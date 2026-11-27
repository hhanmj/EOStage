import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

export const OUTPUTS = Object.freeze({
  rust: "crates/insar-core/src/generated/contracts.rs",
  typescript: "ui/src/lib/generated/v3-contracts.ts",
});

export const AGENT_OUTPUTS = Object.freeze({
  rust: "crates/insar-agent/src/generated_catalog.rs",
  typescript: "ui/src/lib/generated/agent-tool-catalog.ts",
});

const ALLOWED_KEYWORDS = new Set([
  "$schema",
  "$id",
  "$defs",
  "$ref",
  "title",
  "description",
  "type",
  "properties",
  "required",
  "additionalProperties",
  "default",
  "items",
  "minItems",
  "maxItems",
  "enum",
  "const",
  "minimum",
  "maximum",
  "minLength",
  "maxLength",
  "pattern",
  "format",
  "deprecated",
  "x-insar-python-type",
  "x-insar-opaque-object",
  "x-insar-contract-version",
  "x-insar-protocol-major",
  "x-insar-protocol-minor",
  "x-insar-protocol-minimum-minor",
  "x-insar-nullable",
  "x-insar-strict",
  "x-insar-sensitive-key-policy",
  "x-insar-validation",
]);

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function valueMatchesType(value, type) {
  switch (type) {
    case "null":
      return value === null;
    case "object":
      return isPlainObject(value);
    case "array":
      return Array.isArray(value);
    case "string":
      return typeof value === "string";
    case "integer":
      return Number.isInteger(value);
    case "number":
      return typeof value === "number" && Number.isFinite(value);
    case "boolean":
      return typeof value === "boolean";
    default:
      return false;
  }
}

function declaredTypes(node, pointer) {
  if (node.type === undefined) {
    return [];
  }
  const types = Array.isArray(node.type) ? node.type : [node.type];
  const allowedTypes = new Set([
    "object",
    "array",
    "string",
    "integer",
    "number",
    "boolean",
    "null",
  ]);
  if (types.some((type) => !allowedTypes.has(type))) {
    throw new Error(`${pointer}/type: unsupported declared type`);
  }
  if (new Set(types).size !== types.length) {
    throw new Error(`${pointer}/type: duplicate union member`);
  }
  if (types.length > 1) {
    const nonNullTypes = types.filter((type) => type !== "null");
    if (types.length !== 2 || nonNullTypes.length !== 1) {
      throw new Error(`${pointer}/type: ambiguous union`);
    }
  }
  return types;
}

function walkSchema(node, pointer, references) {
  if (!isPlainObject(node)) {
    throw new Error(`${pointer}: schema node must be an object`);
  }

  for (const key of Object.keys(node)) {
    if (!ALLOWED_KEYWORDS.has(key)) {
      throw new Error(`${pointer}: unsupported schema keyword: ${key}`);
    }
  }

  if (node.$ref !== undefined) {
    if (typeof node.$ref !== "string") {
      throw new Error(`${pointer}/$ref: must be a string`);
    }
    references.push({ reference: node.$ref, node, pointer });
  }

  const types = declaredTypes(node, pointer);
  if (
    node.const !== undefined
    && (types.length === 0 || !types.some((type) => valueMatchesType(node.const, type)))
  ) {
    throw new Error(`${pointer}/const: const does not match declared type`);
  }
  if (node.enum !== undefined) {
    if (!Array.isArray(node.enum) || node.enum.length === 0) {
      throw new Error(`${pointer}/enum: must be a non-empty array`);
    }
    const seen = new Set();
    for (const value of node.enum) {
      const rendered = JSON.stringify(value);
      if (rendered === undefined || seen.has(rendered)) {
        throw new Error(`${pointer}/enum: duplicate enum value`);
      }
      seen.add(rendered);
      if (types.length === 0 || !types.some((type) => valueMatchesType(value, type))) {
        throw new Error(`${pointer}/enum: enum value does not match declared type`);
      }
    }
  }
  if (node.minItems !== undefined) {
    if (!Number.isInteger(node.minItems) || node.minItems < 0) {
      throw new Error(`${pointer}/minItems: must be a non-negative integer`);
    }
    if (!types.includes("array")) {
      throw new Error(`${pointer}/minItems: requires an array type`);
    }
  }
  if (node.maxItems !== undefined) {
    if (!Number.isInteger(node.maxItems) || node.maxItems < 0) {
      throw new Error(`${pointer}/maxItems: must be a non-negative integer`);
    }
    if (!types.includes("array")) {
      throw new Error(`${pointer}/maxItems: requires an array type`);
    }
    if (node.minItems !== undefined && node.maxItems < node.minItems) {
      throw new Error(`${pointer}/maxItems: must be greater than or equal to minItems`);
    }
  }
  if (
    types.includes("object")
    && node.properties === undefined
    && node["x-insar-opaque-object"] !== true
  ) {
    throw new Error(
      `${pointer}: object must declare properties or x-insar-opaque-object`,
    );
  }
  if (
    node.default !== undefined
    && !node.$ref
    && !(node.default === null && node["x-insar-nullable"] === true)
    && (types.length === 0
      || !types.some((type) => valueMatchesType(node.default, type)))
  ) {
    throw new Error(`${pointer}/default: default does not match declared type`);
  }
  if (
    node.default !== undefined
    && Array.isArray(node.enum)
    && !node.enum.some((value) => Object.is(value, node.default))
  ) {
    throw new Error(`${pointer}/default: default does not match enum`);
  }

  for (const collectionKey of ["$defs", "properties"]) {
    const collection = node[collectionKey];
    if (collection === undefined) {
      continue;
    }
    if (!isPlainObject(collection)) {
      throw new Error(`${pointer}/${collectionKey}: must be an object`);
    }
    for (const [name, child] of Object.entries(collection)) {
      walkSchema(child, `${pointer}/${collectionKey}/${name}`, references);
    }
  }

  if (node.items !== undefined) {
    walkSchema(node.items, `${pointer}/items`, references);
  }
}

export function validateSchema(schema) {
  const references = [];
  walkSchema(schema, "#", references);
  const definitions = isPlainObject(schema.$defs) ? schema.$defs : {};
  for (const { reference, node, pointer } of references) {
    const match = /^#\/\$defs\/([^/]+)$/.exec(reference);
    if (!match || !Object.hasOwn(definitions, match[1])) {
      throw new Error(`unresolved reference: ${reference}`);
    }
    if (node.default !== undefined) {
      const target = definitions[match[1]];
      const acceptsNull = node["x-insar-nullable"] === true && node.default === null;
      const acceptsEnum = Array.isArray(target.enum) && target.enum.includes(node.default);
      const acceptsType = declaredTypes(target, `#/$defs/${match[1]}`).some(
        (type) => valueMatchesType(node.default, type),
      );
      if (!acceptsNull && !acceptsEnum && !acceptsType) {
        throw new Error(`${pointer}/default: default does not match referenced type`);
      }
    }
  }
  protocolVersions(schema);
}

function contractVersion(schema) {
  const version = schema["x-insar-contract-version"];
  if (typeof version !== "string" || version.length === 0) {
    throw new Error("#: x-insar-contract-version must be a non-empty string");
  }
  return version;
}

function protocolVersions(schema) {
  const major = schema["x-insar-protocol-major"];
  const minor = schema["x-insar-protocol-minor"];
  const minimumMinor = schema["x-insar-protocol-minimum-minor"];
  for (const [name, value] of [
    ["x-insar-protocol-major", major],
    ["x-insar-protocol-minor", minor],
    ["x-insar-protocol-minimum-minor", minimumMinor],
  ]) {
    if (!Number.isInteger(value) || value < 0) {
      throw new Error(`#: ${name} must be a non-negative integer`);
    }
  }
  if (minimumMinor > minor) {
    throw new Error("#: minimum protocol minor must not exceed current minor");
  }
  return { major, minor, minimumMinor };
}

function referenceName(node) {
  return node.$ref?.replace(/^#\/\$defs\//, "");
}

function nonNullType(node) {
  const types = Array.isArray(node.type) ? node.type : [node.type];
  return types.find((type) => type !== "null");
}

function isNullable(node) {
  return node["x-insar-nullable"] === true
    || (Array.isArray(node.type) && node.type.includes("null"));
}

function pascalCase(value) {
  const parts = value.split(/[^A-Za-z0-9]+/).filter(Boolean);
  return parts.map((part) => part[0].toUpperCase() + part.slice(1)).join("");
}

function upperSnakeCase(value) {
  return value.replace(/[^A-Za-z0-9]+/g, "_").toUpperCase();
}

function snakeCase(value) {
  return value
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[^A-Za-z0-9]+/g, "_")
    .toLowerCase();
}

function pythonType(node) {
  if (node.const !== undefined) {
    const rendered = `Literal[${pythonLiteral(node.const)}]`;
    return isNullable(node) ? `${rendered} | None` : rendered;
  }
  if (node.enum !== undefined) {
    const values = node.enum.filter((value) => value !== null);
    const rendered = values.length ? `Literal[${values.map(pythonLiteral).join(", ")}]` : "None";
    return node.enum.includes(null) && values.length ? `${rendered} | None` : rendered;
  }
  if (node.$ref) {
    const rendered = referenceName(node);
    return isNullable(node) ? `${rendered} | None` : rendered;
  }
  let rendered;
  if (node["x-insar-python-type"]) {
    rendered = node["x-insar-python-type"];
  } else {
    switch (nonNullType(node)) {
      case "string":
        rendered = node.format === "path" ? "Path" : node.format === "date-time" ? "datetime" : "str";
        break;
      case "integer":
        rendered = "int";
        break;
      case "number":
        rendered = "float";
        break;
      case "boolean":
        rendered = "bool";
        break;
      case "array":
        rendered = `list[${pythonType(node.items)}]`;
        break;
      case "object":
        if (node["x-insar-opaque-object"] === true) {
          rendered = "dict[str, Any]";
          break;
        }
        throw new Error("inline typed objects are not supported in Python fields");
      default:
        throw new Error("unsupported Python field type");
    }
  }
  return isNullable(node) ? `${rendered} | None` : rendered;
}

function pythonLiteral(value) {
  if (value === null) return "None";
  if (value === true) return "True";
  if (value === false) return "False";
  if (typeof value === "string") return JSON.stringify(value);
  return String(value);
}

function pythonField(name, node) {
  const annotation = pythonType(node);
  const constraints = [];
  if (node["x-insar-strict"] === true) constraints.push("strict=True");
  if (node.minimum !== undefined) constraints.push(`ge=${node.minimum}`);
  if (node.maximum !== undefined) constraints.push(`le=${node.maximum}`);
  if (node.minLength !== undefined && node.format !== "path") constraints.push(`min_length=${node.minLength}`);
  if (node.maxLength !== undefined && node.format !== "path") constraints.push(`max_length=${node.maxLength}`);
  if (node.minItems !== undefined) constraints.push(`min_length=${node.minItems}`);
  if (node.maxItems !== undefined) constraints.push(`max_length=${node.maxItems}`);
  if (node.pattern !== undefined) constraints.push(`pattern=${JSON.stringify(node.pattern)}`);

  if (Array.isArray(node.default)) {
    return `    ${name}: ${annotation} = Field(default_factory=list${constraints.length ? `, ${constraints.join(", ")}` : ""})`;
  }
  if (isPlainObject(node.default)) {
    return `    ${name}: ${annotation} = Field(default_factory=dict${constraints.length ? `, ${constraints.join(", ")}` : ""})`;
  }
  if (node.default !== undefined) {
    const defaultExpression = node.$ref && typeof node.default === "string"
      ? `${referenceName(node)}.${upperSnakeCase(node.default)}`
      : pythonLiteral(node.default);
    if (constraints.length) {
      return `    ${name}: ${annotation} = Field(default=${defaultExpression}, ${constraints.join(", ")})`;
    }
    return `    ${name}: ${annotation} = ${defaultExpression}`;
  }
  if (constraints.length) {
    return `    ${name}: ${annotation} = Field(${constraints.join(", ")})`;
  }
  return `    ${name}: ${annotation}`;
}

function renderPython(schema) {
  const protocol = protocolVersions(schema);
  const lines = [
    "# Generated by scripts/generate_v3_contracts.mjs.",
    "# Source: contracts/v3/provider-contract.schema.json. DO NOT EDIT.",
    "from __future__ import annotations",
    "",
    "import math",
    "import unicodedata",
    "from datetime import datetime",
    "from enum import StrEnum",
    "from pathlib import Path",
    "from typing import Any, Literal",
    "from urllib.parse import urlsplit",
    "",
    "from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator",
    "",
    `V3_CONTRACT_VERSION = ${JSON.stringify(contractVersion(schema))}`,
    `V3_PROTOCOL_MAJOR = ${protocol.major}`,
    `V3_PROTOCOL_MINOR = ${protocol.minor}`,
    `V3_PROTOCOL_MIN_MINOR = ${protocol.minimumMinor}`,
    "",
    "_SENSITIVE_KEYS = frozenset({\"password\", \"token\", \"cookie\", \"authorization\", \"secret\", \"netrc\", \".netrc\", \"environment\", \"env\"})",
    "",
    "def _reject_sensitive_keys(value: Any) -> None:",
    "    if isinstance(value, dict):",
    "        for key, child in value.items():",
    "            normalized = str(key).casefold()",
    "            if normalized in _SENSITIVE_KEYS or normalized.endswith((\"_token\", \"_password\")):",
    "                raise ValueError(f\"sensitive debug-context key is forbidden: {key}\")",
    "            _reject_sensitive_keys(child)",
    "    elif isinstance(value, list):",
    "        for child in value:",
    "            _reject_sensitive_keys(child)",
    "",
    "def _catalogue_position(value: Any, field_name: str) -> tuple[float, float]:",
    "    if not isinstance(value, list) or len(value) < 2:",
    "        raise ValueError(f\"{field_name} positions must contain longitude and latitude\")",
    "    coordinates: list[float] = []",
    "    for coordinate in value:",
    "        if isinstance(coordinate, bool) or not isinstance(coordinate, (int, float)) or not math.isfinite(float(coordinate)):",
    "            raise ValueError(f\"{field_name} positions must be finite numbers\")",
    "        coordinates.append(float(coordinate))",
    "    longitude, latitude = coordinates[:2]",
    "    if not -180.0 <= longitude <= 180.0 or not -90.0 <= latitude <= 90.0:",
    "        raise ValueError(f\"{field_name} must use EPSG:4326 longitude/latitude coordinates\")",
    "    return longitude, latitude",
    "",
    "def _validate_catalogue_linear_ring(value: Any, field_name: str) -> None:",
    "    if not isinstance(value, list) or len(value) < 4:",
    "        raise ValueError(f\"{field_name} polygon rings must contain at least four positions\")",
    "    positions = [_catalogue_position(position, field_name) for position in value]",
    "    if positions[0] != positions[-1]:",
    "        raise ValueError(f\"{field_name} polygon rings must be closed\")",
    "    if len(set(positions[:-1])) < 3:",
    "        raise ValueError(f\"{field_name} polygon rings must contain three distinct positions\")",
    "",
    "def _validate_catalogue_polygon_coordinates(value: Any, field_name: str) -> None:",
    "    if not isinstance(value, list) or not value:",
    "        raise ValueError(f\"{field_name} Polygon coordinates must contain at least one ring\")",
    "    for ring in value:",
    "        _validate_catalogue_linear_ring(ring, field_name)",
    "",
    "def _validate_catalogue_polygonal_geometry(value: Any, field_name: str) -> None:",
    "    if not isinstance(value, dict):",
    "        raise ValueError(f\"{field_name} must be a GeoJSON object\")",
    "    _reject_sensitive_keys(value)",
    "    if \"crs\" in value:",
    "        raise ValueError(f\"{field_name} must not declare a GeoJSON crs member\")",
    "    geometry_type = value.get(\"type\")",
    "    if geometry_type == \"Polygon\":",
    "        _validate_catalogue_polygon_coordinates(value.get(\"coordinates\"), field_name)",
    "    elif geometry_type == \"MultiPolygon\":",
    "        polygons = value.get(\"coordinates\")",
    "        if not isinstance(polygons, list) or not polygons:",
    "            raise ValueError(f\"{field_name} MultiPolygon coordinates must contain at least one polygon\")",
    "        for polygon in polygons:",
    "            _validate_catalogue_polygon_coordinates(polygon, field_name)",
    "    elif geometry_type == \"Feature\":",
    "        _validate_catalogue_polygonal_geometry(value.get(\"geometry\"), field_name)",
    "    elif geometry_type == \"FeatureCollection\":",
    "        features = value.get(\"features\")",
    "        if not isinstance(features, list) or not features:",
    "            raise ValueError(f\"{field_name} FeatureCollection must contain polygonal features\")",
    "        for feature in features:",
    "            if not isinstance(feature, dict) or feature.get(\"type\") != \"Feature\":",
    "                raise ValueError(f\"{field_name} FeatureCollection must contain GeoJSON Features\")",
    "            _validate_catalogue_polygonal_geometry(feature, field_name)",
    "    else:",
    "        raise ValueError(f\"{field_name} must contain Polygon or MultiPolygon topology\")",
    "",
    "def _validate_oauth_bearer_https_href(stable_href: str) -> None:",
    "    if any(character.isspace() or unicodedata.category(character) == \"Cc\" or character == \"\\\\\" for character in stable_href):",
    "        raise ValueError(\"oauth_bearer_https stable_href must not contain control, whitespace, or backslash characters\")",
    "    if not stable_href.startswith(\"https://\"):",
    "        raise ValueError(\"oauth_bearer_https stable_href must be an absolute HTTPS URL\")",
    "    authority = stable_href.removeprefix(\"https://\").split(\"/\", 1)[0]",
    "    if not authority or \"@\" in authority:",
    "        raise ValueError(\"oauth_bearer_https stable_href must have a non-empty host without userinfo\")",
    "    if authority.endswith(\":\"):",
    "        raise ValueError(\"oauth_bearer_https stable_href must not use an empty port\")",
    "    try:",
    "        parsed = urlsplit(stable_href)",
    "        port = parsed.port",
    "    except ValueError as error:",
    "        raise ValueError(\"oauth_bearer_https stable_href must be an absolute HTTPS URL\") from error",
    "    if parsed.scheme != \"https\" or not parsed.hostname:",
    "        raise ValueError(\"oauth_bearer_https stable_href must be an absolute HTTPS URL\")",
    "    if parsed.username is not None or parsed.password is not None:",
    "        raise ValueError(\"oauth_bearer_https stable_href must not contain userinfo\")",
    "    if parsed.query or parsed.fragment:",
    "        raise ValueError(\"oauth_bearer_https stable_href must not contain query or fragment\")",
    "    if port not in (None, 443):",
    "        raise ValueError(\"oauth_bearer_https stable_href must use port 443\")",
    "",
    "class InsarGeneratedModel(BaseModel):",
    "    model_config = ConfigDict(extra=\"forbid\", from_attributes=True, validate_assignment=True)",
  ];

  for (const [name, definition] of Object.entries(schema.$defs)) {
    lines.push("");
    if (definition.enum) {
      lines.push(`class ${name}(StrEnum):`);
      for (const value of definition.enum) {
        lines.push(`    ${upperSnakeCase(value)} = ${JSON.stringify(value)}`);
      }
      continue;
    }
    lines.push(`class ${name}(InsarGeneratedModel):`);
    const properties = Object.entries(definition.properties ?? {});
    if (properties.length === 0) {
      lines.push("    pass");
    } else {
      for (const [fieldName, fieldSchema] of properties) {
        lines.push(pythonField(fieldName, fieldSchema));
      }
    }
    for (const [fieldName, fieldSchema] of properties) {
      if (fieldSchema.format === "path" && fieldSchema.minLength !== undefined) {
        lines.push(
          "",
          `    @field_validator(${JSON.stringify(fieldName)}, mode="before")`,
          "    @classmethod",
          `    def _validate_${fieldName}_path_length(cls, value: Any) -> Any:`,
          `        if value is not None and len(str(value)) < ${fieldSchema.minLength}:`,
          `            raise ValueError(${JSON.stringify(`${fieldName} is too short`)})`,
          "        return value",
        );
      }
    }
    if (definition["x-insar-validation"] === "ordered-bbox") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_ordering(self) -> ${name}:`,
        "        if self.west >= self.east:",
        "            raise ValueError(\"west must be strictly less than east\")",
        "        if self.south >= self.north:",
        "            raise ValueError(\"south must be strictly less than north\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "sidecar-response") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_response_state(self) -> ${name}:`,
        "        if self.ok and self.error is not None:",
        "            raise ValueError(\"successful response cannot contain an error\")",
        "        if not self.ok and self.error is None:",
        "            raise ValueError(\"failed response requires an error\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "asf-search-stats") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_search_diagnostics(self) -> ${name}:`,
        "        diagnostics = [",
        "            self.fetched_record_count, self.replayed_record_count,",
        "            self.mapped_record_count, self.mapping_error_count,",
        "            self.aggregated_asset_record_count, self.unique_provider_product_count,",
        "            self.asset_count, self.identity_conflict_count, self.truncated,",
        "        ]",
        "        if any(value is not None for value in diagnostics) and any(value is None for value in diagnostics):",
        "            raise ValueError(\"ASF search diagnostics must be supplied together\")",
        "        if all(value is not None for value in diagnostics):",
        "            if self.returned_count > self.unique_provider_product_count:",
        "                raise ValueError(\"returned_count must not exceed unique_provider_product_count\")",
        "            if self.unique_provider_product_count > self.mapped_record_count:",
        "                raise ValueError(\"unique_provider_product_count must not exceed mapped_record_count\")",
        "            if self.replayed_record_count > self.fetched_record_count:",
        "                raise ValueError(\"replayed_record_count must not exceed fetched_record_count\")",
        "            if self.asset_count > self.aggregated_asset_record_count:",
        "                raise ValueError(\"asset_count must not exceed aggregated_asset_record_count\")",
        "            if self.mapping_error_count != len(self.item_errors):",
        "                raise ValueError(\"mapping_error_count must equal item_errors length\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "asset-access-descriptor") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_durable_access(self) -> ${name}:`,
        "        if self.requires_credentials != (self.credential_profile is not None):",
        "            raise ValueError(\"credential_profile presence must match requires_credentials\")",
        "        if self.scheme.value == \"oauth_bearer_https\" and not self.requires_credentials:",
        "            raise ValueError(\"oauth_bearer_https requires credentials and a credential_profile\")",
        "        if self.scheme.value == \"public_https\" and self.requires_credentials:",
        "            raise ValueError(\"public_https must not require credentials\")",
        "        if self.stable_href is not None:",
        "            href = self.stable_href.strip()",
        "            if not href or \"\\x00\" in href or \"?\" in href or \"#\" in href:",
        "                raise ValueError(\"stable_href must be a durable locator without query or fragment\")",
        "            authority = href.split(\"://\", 1)[1].split(\"/\", 1)[0] if \"://\" in href else \"\"",
        "            if \"@\" in authority:",
        "                raise ValueError(\"stable_href must not contain userinfo\")",
        "        if self.scheme.value == \"oauth_bearer_https\":",
        "            if self.stable_href is None:",
        "                raise ValueError(\"oauth_bearer_https requires an absolute HTTPS stable_href\")",
        "            _validate_oauth_bearer_https_href(self.stable_href)",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "provider-asset") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_native_roles(self) -> ${name}:`,
        "        for native_role in self.native_roles:",
        "            if not native_role or any(unicodedata.category(character) == \"Cc\" for character in native_role):",
        "                raise ValueError(\"native_roles entries must be non-empty and contain no control characters\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "provider-product") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_product_assets_and_footprint(self) -> ${name}:`,
        "        if self.footprint_geojson is not None:",
        "            _validate_catalogue_polygonal_geometry(self.footprint_geojson, \"footprint_geojson\")",
        "        asset_keys = [asset.asset_key for asset in self.assets if asset.asset_key is not None]",
        "        stable_asset_keys = [asset.stable_asset_key for asset in self.assets if asset.stable_asset_key is not None]",
        "        if len(asset_keys) != len(set(asset_keys)):",
        "            raise ValueError(\"asset_key values must be unique within one product\")",
        "        if len(stable_asset_keys) != len(set(stable_asset_keys)):",
        "            raise ValueError(\"stable_asset_key values must be unique within one product\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "resolved-preview-asset") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_renderable_source_role(self) -> ${name}:`,
        "        if self.source_asset_role not in {PreviewAssetRole.PREVIEW, PreviewAssetRole.BROWSE, PreviewAssetRole.THUMBNAIL}:",
        "            raise ValueError(\"source_asset_role must be renderable Preview evidence\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "catalog-search-query") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_catalogue_aoi(self) -> ${name}:`,
        "        if self.aoi_geojson is not None:",
        "            _validate_catalogue_polygonal_geometry(self.aoi_geojson, \"aoi_geojson\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "catalog-search-stats") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_catalogue_counts(self) -> ${name}:`,
        "        if self.mapped_record_count > self.fetched_record_count:",
        "            raise ValueError(\"mapped_record_count must not exceed fetched_record_count\")",
        "        if self.mapped_record_count != (self.unique_provider_product_count + self.exact_duplicate_record_count):",
        "            raise ValueError(\"mapped_record_count must equal unique_provider_product_count plus exact_duplicate_record_count\")",
        "        if self.returned_count > self.unique_provider_product_count:",
        "            raise ValueError(\"returned_count must not exceed unique_provider_product_count\")",
        "        if self.exact_matched_count > self.unique_provider_product_count:",
        "            raise ValueError(\"exact_matched_count must not exceed unique_provider_product_count\")",
        "        if self.returned_count > self.exact_matched_count:",
        "            raise ValueError(\"returned_count must not exceed exact_matched_count\")",
        "        if self.matched_count is not None and self.returned_count > self.matched_count:",
        "            raise ValueError(\"returned_count must not exceed matched_count\")",
        "        if not self.working_set_satisfied:",
        "            raise ValueError(\"working_set_satisfied must confirm successful bounded completion\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "application-search-progress") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_application_progress(self) -> ${name}:`,
        "        terminal_stage = self.stage.value in {\"completed\", \"failed\", \"cancelled\"}",
        "        if self.terminal != terminal_stage:",
        "            raise ValueError(\"terminal must match the application progress stage\")",
        "        ordered_counts = [",
        "            (self.mapped_count, self.raw_fetched_count, \"mapped_count must not exceed raw_fetched_count\"),",
        "            (self.exact_matched_count, self.mapped_count, \"exact_matched_count must not exceed mapped_count\"),",
        "            (self.admitted_count, self.exact_matched_count, \"admitted_count must not exceed exact_matched_count\"),",
        "            (self.admitted_count, self.requested_limit, \"admitted_count must not exceed requested_limit\"),",
        "        ]",
        "        for lower, upper, message in ordered_counts:",
        "            if lower is not None and upper is not None and lower > upper:",
        "                raise ValueError(message)",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "catalog-search-response") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_catalogue_page(self) -> ${name}:`,
        "        if self.stats.fetched_record_count != self.stats.mapped_record_count + self.stats.profile_filtered_record_count + len(self.item_errors):",
        "            raise ValueError(\"fetched_record_count must equal mapped_record_count plus profile_filtered_record_count plus item_errors length\")",
        "        if self.stats.returned_count != len(self.products):",
        "            raise ValueError(\"returned_count must equal products length\")",
        "        if self.stats.truncated != (self.next_cursor is not None):",
        "            raise ValueError(\"truncated must match next_cursor presence\")",
        "        if any(product.provider_id != self.collection.provider_id for product in self.products):",
        "            raise ValueError(\"product provider IDs must match the response collection\")",
        "        product_ids = [product.product_id for product in self.products]",
        "        if len(product_ids) != len(set(product_ids)):",
        "            raise ValueError(\"product IDs must be unique within one response page\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "frozen-asset-access") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_checksum_pair(self) -> ${name}:`,
        "        if (self.checksum_algorithm is None) != (self.checksum_value is None):",
        "            raise ValueError(\"checksum_algorithm presence must match checksum_value\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "optional-checksum-pair") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_checksum_pair(self) -> ${name}:`,
        "        if (self.checksum_algorithm is None) != (self.checksum_value is None):",
        "            raise ValueError(\"checksum_algorithm presence must match checksum_value\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "asset-execution-policy") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_authority_shape(self) -> ${name}:`,
        "        if self.authority.value == \"legacy\":",
        "            if self.policy_id != \"legacy.v1\":",
        "                raise ValueError(\"legacy policy_id must be legacy.v1\")",
        "        else:",
        "            if self.policy_id == \"legacy.v1\":",
        "                raise ValueError(\"resolver policy_id must not be legacy.v1\")",
        "            if self.collection_profile_id is None:",
        "                raise ValueError(\"resolver collection_profile_id is required\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "orbit-file-metadata") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_validity_window(self) -> ${name}:`,
        "        if \"\\x00\" in self.file_name or \"/\" in self.file_name or \"\\\\\" in self.file_name:",
        "            raise ValueError(\"file_name must be a basename\")",
        "        if self.validity_stop_unix_seconds <= self.validity_start_unix_seconds:",
        "            raise ValueError(\"validity stop must be after start\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "orbit-parse-request") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_file_names(self) -> ${name}:`,
        "        for file_name in self.file_names:",
        "            if not file_name or \"\\x00\" in file_name or \"/\" in file_name or \"\\\\\" in file_name:",
        "                raise ValueError(\"file_names must contain basenames only\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "frozen-search-stats") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_source_attempts(self) -> ${name}:`,
        "        if self.source_attempts:",
        "            for attempt in self.source_attempts[:-1]:",
        "                if attempt.outcome != FrozenSearchSourceAttemptOutcome.FALLBACK or attempt.fallback_reason is None:",
        "                    raise ValueError(\"source attempts before the final success must be bounded fallbacks\")",
        "            final = self.source_attempts[-1]",
        "            if final.outcome != FrozenSearchSourceAttemptOutcome.SUCCEEDED or final.fallback_reason is not None:",
        "                raise ValueError(\"source attempts must end in one success without a fallback reason\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "frozen-search-batch") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_frozen_batch(self) -> ${name}:`,
        "        if self.aoi_geojson is not None:",
        "            geometry_type = str(self.aoi_geojson.get(\"type\") or \"\")",
        "            if geometry_type not in {\"Polygon\", \"MultiPolygon\", \"Feature\", \"FeatureCollection\", \"GeometryCollection\"}:",
        "                raise ValueError(\"aoi_geojson must contain real polygon topology\")",
        "        candidate_ids = [item.item_id for item in self.candidates]",
        "        if len(candidate_ids) != len(set(candidate_ids)):",
        "            raise ValueError(\"candidate item IDs must be unique\")",
        "        if any(item.provider_id != self.provider_id for item in self.candidates):",
        "            raise ValueError(\"candidate provider IDs must match the batch provider\")",
        "        if self.search_stats is not None and self.search_stats.source_attempts:",
        "            if self.search_stats.source_attempts[-1].provider_id != self.provider_id:",
        "                raise ValueError(\"final source attempt provider must match the batch provider\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "frozen-task-snapshot") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_frozen_task(self) -> ${name}:`,
        "        if self.aoi_geojson is not None:",
        "            geometry_type = str(self.aoi_geojson.get(\"type\") or \"\")",
        "            if geometry_type not in {\"Polygon\", \"MultiPolygon\", \"Feature\", \"FeatureCollection\", \"GeometryCollection\"}:",
        "                raise ValueError(\"aoi_geojson must contain real polygon topology\")",
        "        candidate_ids = [item.item_id for item in self.candidates]",
        "        if len(candidate_ids) != len(set(candidate_ids)):",
        "            raise ValueError(\"candidate item IDs must be unique\")",
        "        if any(item.provider_id != self.provider_id for item in self.candidates):",
        "            raise ValueError(\"candidate provider IDs must match the task provider\")",
        "        selected = self.initial_selected_item_ids",
        "        if len(selected) != len(set(selected)) or not set(selected).issubset(candidate_ids):",
        "            raise ValueError(\"initial selected item IDs must be a unique candidate subset\")",
        "        if not str(self.output_root).strip():",
        "            raise ValueError(\"output_root must not be empty\")",
        "        return self",
      );
    }
    if (definition["x-insar-validation"] === "task-runtime-snapshot") {
      lines.push(
        "",
        "    @model_validator(mode=\"after\")",
        `    def _validate_runtime_state(self) -> ${name}:`,
        "        selected = set(self.selected_item_ids)",
        "        if len(selected) != len(self.selected_item_ids) or len(set(self.checked_item_ids)) != len(self.checked_item_ids):",
        "            raise ValueError(\"selected and checked item IDs must be unique\")",
        "        lifecycle_lists = [",
        "            self.active_item_ids, self.paused_item_ids, self.queued_item_ids,",
        "            self.completed_item_ids, self.failed_item_ids, self.skipped_item_ids,",
        "            self.cancelled_item_ids,",
        "        ]",
        "        lifecycle = [set(values) for values in lifecycle_lists]",
        "        if any(len(values) != len(raw) for values, raw in zip(lifecycle, lifecycle_lists, strict=True)):",
        "            raise ValueError(\"task lifecycle item IDs must be unique\")",
        "        if any(not values.issubset(selected) for values in lifecycle):",
        "            raise ValueError(\"task lifecycle item IDs must be selected\")",
        "        if any(not left.isdisjoint(right) for index, left in enumerate(lifecycle) for right in lifecycle[index + 1:]):",
        "            raise ValueError(\"task lifecycle sets must be mutually exclusive\")",
        "        if len(self.active_item_ids) > self.concurrency:",
        "            raise ValueError(\"active item count exceeds concurrency\")",
        "        return self",
      );
    }
    for (const [fieldName, fieldSchema] of properties) {
      if (fieldSchema["x-insar-validation"] === "nonblank-aoi-name") {
        lines.push(
          "",
          `    @field_validator(${JSON.stringify(fieldName)})`,
          "    @classmethod",
          `    def _validate_${fieldName}(cls, value: str) -> str:`,
          "        if not value.strip():",
          `            raise ValueError(${JSON.stringify(`${fieldName} must not be blank`)})`,
          "        return value",
        );
      }
      if (fieldSchema["x-insar-sensitive-key-policy"] === true) {
        lines.push(
          "",
          `    @field_validator(${JSON.stringify(fieldName)})`,
          "    @classmethod",
          `    def _validate_${fieldName}(cls, value: dict[str, Any]) -> dict[str, Any]:`,
          "        _reject_sensitive_keys(value)",
          "        return value",
        );
      }
    }
  }
  return `${lines.join("\n")}\n`;
}

function rustType(node) {
  if (node.$ref) {
    const rendered = referenceName(node);
    return isNullable(node) ? `Option<${rendered}>` : rendered;
  }
  let rendered;
  switch (nonNullType(node)) {
    case "string":
      rendered = node.format === "path" ? "PathBuf" : "String";
      break;
    case "integer":
      rendered = node.minimum !== undefined && node.minimum >= 0 ? "u64" : "i64";
      break;
    case "number":
      rendered = "f64";
      break;
    case "boolean":
      rendered = "bool";
      break;
    case "array":
      rendered = `Vec<${rustType(node.items)}>`;
      break;
    case "object":
      if (node["x-insar-opaque-object"] === true) {
        rendered = "Map<String, Value>";
        break;
      }
      throw new Error("inline typed objects are not supported in Rust fields");
    default:
      throw new Error("unsupported Rust field type");
  }
  return isNullable(node) ? `Option<${rendered}>` : rendered;
}

function rustLiteral(value) {
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  throw new Error("unsupported Rust const literal");
}

function rustEnumValidationLines(node, comparisonValue, pathExpression, indent) {
  const allowedValues = node.enum.filter((value) => value !== null).map(rustLiteral);
  const errorLines = () => {
    if (pathExpression.startsWith('"')) {
      const path = JSON.parse(pathExpression);
      return [`${indent}    return Err(${JSON.stringify(`${path} must be one of the declared values`)}.to_string());`];
    }
    return [
      `${indent}    return Err(format!(`,
      `${indent}        "{} must be one of the declared values",`,
      `${indent}        ${pathExpression}`,
      `${indent}    ));`,
    ];
  };
  if (allowedValues.length === 0) {
    return [
      ...errorLines().map((line) => line.replace(`${indent}    `, `${indent}`)),
    ];
  }
  const inlineCondition = `if ![${allowedValues.join(", ")}].contains(&${comparisonValue}) {`;
  const lines = [];
  if (`${indent}${inlineCondition}`.length > 100) {
    const inlineArray = `[${allowedValues.join(", ")}]`;
    if (`${indent}if !${inlineArray}`.length <= 80 && inlineArray.length <= 60) {
      lines.push(`${indent}if !${inlineArray}`);
      lines.push(`${indent}    .contains(&${comparisonValue})`);
    } else {
      lines.push(`${indent}if ![`);
      lines.push(...allowedValues.map((value) => `${indent}    ${value},`));
      lines.push(`${indent}]`);
      lines.push(`${indent}.contains(&${comparisonValue})`);
    }
    lines.push(`${indent}{`);
  } else {
    lines.push(`${indent}${inlineCondition}`);
  }
  lines.push(...errorLines());
  lines.push(`${indent}}`);
  return lines;
}

function rustNestedValidationLines(valueExpression, pathExpression, indent) {
  return [
    `${indent}${valueExpression}`,
    `${indent}    .validate()`,
    `${indent}    .map_err(|error| format!("{}.{}", ${pathExpression}, error))?;`,
  ];
}

function rustArrayItemValidationLines(node, schema, collectionExpression, indent, parentPath) {
  const lines = [
    `${indent}for (index, value) in ${collectionExpression}.iter().enumerate() {`,
    `${indent}    let value_path = format!("{}[{}]", ${parentPath}, index);`,
  ];
  let valueIndent = `${indent}    `;
  if (isNullable(node)) {
    lines.push(`${valueIndent}if let Some(value) = value {`);
    valueIndent += "    ";
  }
  if (nonNullType(node) === "array") {
    lines.push(
      ...rustArrayItemValidationLines(node.items, schema, "value", valueIndent, "value_path"),
    );
  } else {
    if (Array.isArray(node.enum)) {
      const comparisonValue = nonNullType(node) === "string" ? "value.as_str()" : "*value";
      lines.push(...rustEnumValidationLines(node, comparisonValue, "value_path", valueIndent));
    }
    if (node.$ref) {
      const target = schema.$defs[referenceName(node)];
      if (target?.type === "object") {
        lines.push(...rustNestedValidationLines("value", "value_path", valueIndent));
      }
    }
  }
  if (isNullable(node)) {
    if (Array.isArray(node.enum) && !node.enum.includes(null)) {
      lines.push(`${indent}    } else {`);
      lines.push(
        `${indent}        return Err(format!("{} must be one of the declared values", value_path));`,
      );
    }
    lines.push(`${indent}    }`);
  }
  lines.push(`${indent}}`);
  return lines;
}

function requiresRustArrayItemValidation(node, schema) {
  if (Array.isArray(node.enum)) return true;
  if (node.$ref) {
    return schema.$defs[referenceName(node)]?.type === "object";
  }
  return nonNullType(node) === "array"
    && requiresRustArrayItemValidation(node.items, schema);
}

function rustDefaultAttribute(owner, fieldName, node, defaultFunctions) {
  if (node.default === undefined) return [];
  if (
    node.default === null
    || node.default === false
    || node.default === 0
    || node.default === ""
    || Array.isArray(node.default)
    || isPlainObject(node.default)
  ) {
    return ["    #[serde(default)]"];
  }
  const functionName = `default_${snakeCase(owner)}_${snakeCase(fieldName)}`;
  const type = rustType(node);
  let expression;
  if (node.$ref && typeof node.default === "string") {
    expression = `${referenceName(node)}::${pascalCase(node.default)}`;
  } else if (typeof node.default === "string") {
    expression = `${JSON.stringify(node.default)}.to_string()`;
  } else if (typeof node.default === "boolean" || typeof node.default === "number") {
    expression = String(node.default);
  } else {
    throw new Error(`unsupported Rust default for ${owner}.${fieldName}`);
  }
  defaultFunctions.push(
    `fn ${functionName}() -> ${type} {`,
    `    ${expression}`,
    "}",
  );
  return [`    #[serde(default = "${functionName}")]`];
}

function rustConstraintLines(fieldName, node, schema) {
  const lines = [];
  const field = `self.${rustIdentifier(fieldName)}`;
  const optional = isNullable(node);
  const valueExpression = optional ? "value" : field;
  const indent = optional ? "            " : "        ";
  const finiteNumber = nonNullType(node) === "number";
  const hasEnum = Array.isArray(node.enum);
  const hasScalarConstraint = finiteNumber
    || node.minimum !== undefined
    || node.maximum !== undefined
    || node.minLength !== undefined
    || node.maxLength !== undefined
    || node.minItems !== undefined
    || node.maxItems !== undefined
    || node.const !== undefined
    || hasEnum;
  if (optional && hasScalarConstraint) {
    lines.push(`        if let Some(value) = &${field} {`);
  }
  if (finiteNumber) {
    lines.push(`${indent}if !${optional ? "value" : field}.is_finite() {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must be finite`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (node.minimum !== undefined) {
    lines.push(`${indent}if ${optional ? "*value" : field} < ${node.minimum} as _ {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must be >= ${node.minimum}`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (node.maximum !== undefined) {
    lines.push(`${indent}if ${optional ? "*value" : field} > ${node.maximum} as _ {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must be <= ${node.maximum}`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (node.minLength !== undefined) {
    if (node.format === "path") {
      const pathReceiver = optional
        ? [`${indent}if value`]
        : [`${indent}if self`, `${indent}    .${rustIdentifier(fieldName)}`];
      lines.push(
        ...pathReceiver,
        `${indent}    .as_os_str()`,
        `${indent}    .to_string_lossy()`,
        `${indent}    .chars()`,
        `${indent}    .count()`,
        `${indent}    < ${node.minLength}`,
        `${indent}{`,
        `${indent}    return Err(${JSON.stringify(`${fieldName} is too short`)}.to_string());`,
        `${indent}}`,
      );
    } else {
      const lengthExpression = `${optional ? "value" : field}.chars().count()`;
      lines.push(`${indent}if ${lengthExpression} < ${node.minLength} {`);
      lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} is too short`)}.to_string());`);
      lines.push(`${indent}}`);
    }
  }
  if (node.maxLength !== undefined) {
    if (node.format === "path") {
      const pathReceiver = optional
        ? [`${indent}if value`]
        : [`${indent}if self`, `${indent}    .${rustIdentifier(fieldName)}`];
      lines.push(
        ...pathReceiver,
        `${indent}    .as_os_str()`,
        `${indent}    .to_string_lossy()`,
        `${indent}    .chars()`,
        `${indent}    .count()`,
        `${indent}    > ${node.maxLength}`,
        `${indent}{`,
        `${indent}    return Err(${JSON.stringify(`${fieldName} is too long`)}.to_string());`,
        `${indent}}`,
      );
    } else {
      const lengthExpression = `${optional ? "value" : field}.chars().count()`;
      lines.push(`${indent}if ${lengthExpression} > ${node.maxLength} {`);
      lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} is too long`)}.to_string());`);
      lines.push(`${indent}}`);
    }
  }
  if (node.minItems > 0) {
    const lengthConstraint = node.minItems === 1
      ? `${valueExpression}.is_empty()`
      : `${valueExpression}.len() < ${node.minItems}`;
    lines.push(`${indent}if ${lengthConstraint} {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must contain at least ${node.minItems} item${node.minItems === 1 ? "" : "s"}`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (node.maxItems !== undefined) {
    lines.push(`${indent}if ${valueExpression}.len() > ${node.maxItems} {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must contain at most ${node.maxItems} items`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (node.const !== undefined) {
    const constant = rustLiteral(node.const);
    const comparisonValue = nonNullType(node) === "string"
      ? `${optional ? "value" : field}.as_str()`
      : optional ? "*value" : field;
    lines.push(`${indent}if ${comparisonValue} != ${constant} {`);
    lines.push(`${indent}    return Err(${JSON.stringify(`${fieldName} must equal ${node.const}`)}.to_string());`);
    lines.push(`${indent}}`);
  }
  if (hasEnum) {
    const comparisonValue = nonNullType(node) === "string"
      ? `${optional ? "value" : field}.as_str()`
      : optional ? "*value" : field;
    lines.push(...rustEnumValidationLines(node, comparisonValue, JSON.stringify(fieldName), indent));
  }
  if (optional && hasScalarConstraint) {
    if (hasEnum && !node.enum.includes(null)) {
      lines.push("        } else {");
      lines.push(
        `            return Err(${JSON.stringify(`${fieldName} must be one of the declared values`)}.to_string());`,
      );
    }
    lines.push("        }");
  }

  if (node.$ref) {
    const target = schema.$defs[referenceName(node)];
    if (target?.type === "object") {
      if (optional) {
        lines.push(
          `        if let Some(value) = &${field} {`,
          ...rustNestedValidationLines("value", JSON.stringify(fieldName), "            "),
          "        }",
        );
      } else {
        lines.push(...rustNestedValidationLines(field, JSON.stringify(fieldName), "        "));
      }
    }
  }
  if (
    nonNullType(node) === "array"
    && requiresRustArrayItemValidation(node.items, schema)
  ) {
    if (optional) {
      lines.push(`        if let Some(value) = &${field} {`);
      lines.push(...rustArrayItemValidationLines(node.items, schema, "value", "            ", JSON.stringify(fieldName)));
      lines.push("        }");
    } else {
      lines.push(...rustArrayItemValidationLines(node.items, schema, field, "        ", JSON.stringify(fieldName)));
    }
  }
  if (node["x-insar-sensitive-key-policy"] === true) {
    lines.push(`        validate_debug_context(&${field})?;`);
  }
  return lines;
}

const RUST_RESERVED_WORDS = new Set([
  "as", "break", "const", "continue", "crate", "else", "enum", "extern",
  "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
  "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
  "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
  "while", "async", "await", "dyn", "abstract", "become", "box", "do",
  "final", "macro", "override", "priv", "typeof", "unsized", "virtual",
  "yield", "try",
]);

function rustIdentifier(value) {
  return RUST_RESERVED_WORDS.has(value) ? `r#${value}` : value;
}

function rustValidationLines(name, definition, schema) {
  const lines = ["", `impl ${name} {`, "    pub fn validate(&self) -> Result<(), String> {"];
  for (const [fieldName, fieldSchema] of Object.entries(definition.properties ?? {})) {
    lines.push(...rustConstraintLines(fieldName, fieldSchema, schema));
    if (fieldSchema["x-insar-validation"] === "nonblank-aoi-name") {
      lines.push(
        `        if self.${rustIdentifier(fieldName)}.trim().is_empty() {`,
        `            return Err(${JSON.stringify(`${fieldName} must not be blank`)}.to_string());`,
        "        }",
      );
    }
  }
  if (definition["x-insar-validation"] === "ordered-bbox") {
    lines.push(
      "        if self.west >= self.east {",
      "            return Err(\"west must be strictly less than east\".to_string());",
      "        }",
      "        if self.south >= self.north {",
      "            return Err(\"south must be strictly less than north\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "sidecar-response") {
    lines.push(
      "        if self.ok && self.error.is_some() {",
      "            return Err(\"successful response cannot contain an error\".to_string());",
      "        }",
      "        if !self.ok && self.error.is_none() {",
      "            return Err(\"failed response requires an error\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "asf-search-stats") {
    lines.push(
      "        let diagnostic_count = [",
      "            self.fetched_record_count.is_some(),",
      "            self.replayed_record_count.is_some(),",
      "            self.mapped_record_count.is_some(),",
      "            self.mapping_error_count.is_some(),",
      "            self.aggregated_asset_record_count.is_some(),",
      "            self.unique_provider_product_count.is_some(),",
      "            self.asset_count.is_some(),",
      "            self.identity_conflict_count.is_some(),",
      "            self.truncated.is_some(),",
      "        ]",
      "        .into_iter()",
      "        .filter(|present| *present)",
      "        .count();",
      "        if diagnostic_count != 0 && diagnostic_count != 9 {",
      "            return Err(\"ASF search diagnostics must be supplied together\".to_string());",
      "        }",
      "        if diagnostic_count == 9 {",
      "            let fetched = self.fetched_record_count.expect(\"diagnostics checked\");",
      "            let replayed = self.replayed_record_count.expect(\"diagnostics checked\");",
      "            let mapped = self.mapped_record_count.expect(\"diagnostics checked\");",
      "            let unique = self",
      "                .unique_provider_product_count",
      "                .expect(\"diagnostics checked\");",
      "            let assets = self.asset_count.expect(\"diagnostics checked\");",
      "            let aggregated_assets = self",
      "                .aggregated_asset_record_count",
      "                .expect(\"diagnostics checked\");",
      "            if self.returned_count > unique {",
      "                return Err(",
      "                    \"returned_count must not exceed unique_provider_product_count\".to_string(),",
      "                );",
      "            }",
      "            if unique > mapped {",
      "                return Err(",
      "                    \"unique_provider_product_count must not exceed mapped_record_count\".to_string(),",
      "                );",
      "            }",
      "            if replayed > fetched {",
      "                return Err(",
      "                    \"replayed_record_count must not exceed fetched_record_count\".to_string()",
      "                );",
      "            }",
      "            if assets > aggregated_assets {",
      "                return Err(\"asset_count must not exceed aggregated_asset_record_count\".to_string());",
      "            }",
      "            if self.mapping_error_count.expect(\"diagnostics checked\")",
      "                != self.item_errors.len() as u64",
      "            {",
      "                return Err(\"mapping_error_count must equal item_errors length\".to_string());",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "asset-access-descriptor") {
    lines.push(
      "        if self.requires_credentials != self.credential_profile.is_some() {",
      "            return Err(\"credential_profile presence must match requires_credentials\".to_string());",
      "        }",
      "        if matches!(self.scheme, AssetAccessScheme::OauthBearerHttps) && !self.requires_credentials",
      "        {",
      "            return Err(",
      "                \"oauth_bearer_https requires credentials and a credential_profile\".to_string(),",
      "            );",
      "        }",
      "        if matches!(self.scheme, AssetAccessScheme::PublicHttps) && self.requires_credentials {",
      "            return Err(\"public_https must not require credentials\".to_string());",
      "        }",
      "        if let Some(stable_href) = &self.stable_href {",
      "            let stable_href = stable_href.trim();",
      "            if stable_href.is_empty()",
      "                || stable_href.contains('\\0')",
      "                || stable_href.contains('?')",
      "                || stable_href.contains('#')",
      "            {",
      "                return Err(",
      "                    \"stable_href must be a durable locator without query or fragment\".to_string(),",
      "                );",
      "            }",
      "            if stable_href",
      "                .split_once(\"://\")",
      "                .map(|(_, rest)| rest.split('/').next().unwrap_or(\"\").contains('@'))",
      "                .unwrap_or(false)",
      "            {",
      "                return Err(\"stable_href must not contain userinfo\".to_string());",
      "            }",
      "        }",
      "        if matches!(self.scheme, AssetAccessScheme::OauthBearerHttps) {",
      "            let stable_href = self.stable_href.as_deref().ok_or_else(|| {",
      "                \"oauth_bearer_https requires an absolute HTTPS stable_href\".to_string()",
      "            })?;",
      "            validate_oauth_bearer_https_href(stable_href)?;",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "provider-asset") {
    lines.push(
      "        for native_role in &self.native_roles {",
      "            if native_role.is_empty() || native_role.chars().any(char::is_control) {",
      "                return Err(",
      "                    \"native_roles entries must be non-empty and contain no control characters\"",
      "                        .to_string(),",
      "                );",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "provider-product") {
    lines.push(
      "        if let Some(footprint_geojson) = &self.footprint_geojson {",
      "            validate_catalogue_polygonal_geojson(footprint_geojson, \"footprint_geojson\")?;",
      "        }",
      "        let asset_keys: HashSet<&str> = self",
      "            .assets",
      "            .iter()",
      "            .filter_map(|asset| asset.asset_key.as_deref())",
      "            .collect();",
      "        if asset_keys.len()",
      "            != self",
      "                .assets",
      "                .iter()",
      "                .filter(|asset| asset.asset_key.is_some())",
      "                .count()",
      "        {",
      "            return Err(\"asset_key values must be unique within one product\".to_string());",
      "        }",
      "        let stable_asset_keys: HashSet<&str> = self",
      "            .assets",
      "            .iter()",
      "            .filter_map(|asset| asset.stable_asset_key.as_deref())",
      "            .collect();",
      "        if stable_asset_keys.len()",
      "            != self",
      "                .assets",
      "                .iter()",
      "                .filter(|asset| asset.stable_asset_key.is_some())",
      "                .count()",
      "        {",
      "            return Err(\"stable_asset_key values must be unique within one product\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "resolved-preview-asset") {
    lines.push(
      "        if !matches!(",
      "            &self.source_asset_role,",
      "            PreviewAssetRole::Preview | PreviewAssetRole::Browse | PreviewAssetRole::Thumbnail",
      "        ) {",
      "            return Err(\"source_asset_role must be renderable Preview evidence\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "catalog-search-query") {
    lines.push(
      "        if let Some(aoi_geojson) = &self.aoi_geojson {",
      "            validate_catalogue_polygonal_geojson(aoi_geojson, \"aoi_geojson\")?;",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "catalog-search-stats") {
    lines.push(
      "        if self.mapped_record_count > self.fetched_record_count {",
      "            return Err(\"mapped_record_count must not exceed fetched_record_count\".to_string());",
      "        }",
      "        if self",
      "            .unique_provider_product_count",
      "            .checked_add(self.exact_duplicate_record_count)",
      "            != Some(self.mapped_record_count)",
      "        {",
      "            return Err(",
      "                \"mapped_record_count must equal unique_provider_product_count plus exact_duplicate_record_count\".to_string(),",
      "            );",
      "        }",
      "        if self.returned_count > self.unique_provider_product_count {",
      "            return Err(\"returned_count must not exceed unique_provider_product_count\".to_string());",
      "        }",
      "        if self.exact_matched_count > self.unique_provider_product_count {",
      "            return Err(",
      "                \"exact_matched_count must not exceed unique_provider_product_count\".to_string(),",
      "            );",
      "        }",
      "        if self.returned_count > self.exact_matched_count {",
      "            return Err(\"returned_count must not exceed exact_matched_count\".to_string());",
      "        }",
      "        if self",
      "            .matched_count",
      "            .is_some_and(|matched_count| self.returned_count > matched_count)",
      "        {",
      "            return Err(\"returned_count must not exceed matched_count\".to_string());",
      "        }",
      "        if !self.working_set_satisfied {",
      "            return Err(",
      "                \"working_set_satisfied must confirm successful bounded completion\".to_string(),",
      "            );",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "application-search-progress") {
    lines.push(
      "        let terminal_stage = matches!(",
      "            self.stage,",
      "            ApplicationSearchProgressStage::Completed",
      "                | ApplicationSearchProgressStage::Failed",
      "                | ApplicationSearchProgressStage::Cancelled",
      "        );",
      "        if self.terminal != terminal_stage {",
      "            return Err(\"terminal must match the application progress stage\".to_string());",
      "        }",
      "        for (lower, upper, message) in [",
      "            (",
      "                self.mapped_count,",
      "                self.raw_fetched_count,",
      "                \"mapped_count must not exceed raw_fetched_count\",",
      "            ),",
      "            (",
      "                self.exact_matched_count,",
      "                self.mapped_count,",
      "                \"exact_matched_count must not exceed mapped_count\",",
      "            ),",
      "            (",
      "                self.admitted_count,",
      "                self.exact_matched_count,",
      "                \"admitted_count must not exceed exact_matched_count\",",
      "            ),",
      "            (",
      "                self.admitted_count,",
      "                self.requested_limit,",
      "                \"admitted_count must not exceed requested_limit\",",
      "            ),",
      "        ] {",
      "            if matches!((lower, upper), (Some(lower), Some(upper)) if lower > upper) {",
      "                return Err(message.to_string());",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "catalog-search-response") {
    lines.push(
      "        if self",
      "            .stats",
      "            .mapped_record_count",
      "            .checked_add(self.stats.profile_filtered_record_count)",
      "            .and_then(|count| count.checked_add(self.item_errors.len() as u64))",
      "            != Some(self.stats.fetched_record_count)",
      "        {",
      "            return Err(",
      "                \"fetched_record_count must equal mapped_record_count plus profile_filtered_record_count plus item_errors length\"",
      "                    .to_string(),",
      "            );",
      "        }",
      "        if self.stats.returned_count != self.products.len() as u64 {",
      "            return Err(\"returned_count must equal products length\".to_string());",
      "        }",
      "        if self.stats.truncated != self.next_cursor.is_some() {",
      "            return Err(\"truncated must match next_cursor presence\".to_string());",
      "        }",
      "        if self",
      "            .products",
      "            .iter()",
      "            .any(|product| product.provider_id != self.collection.provider_id)",
      "        {",
      "            return Err(\"product provider IDs must match the response collection\".to_string());",
      "        }",
      "        let product_ids: HashSet<&str> = self",
      "            .products",
      "            .iter()",
      "            .map(|product| product.product_id.as_str())",
      "            .collect();",
      "        if product_ids.len() != self.products.len() {",
      "            return Err(\"product IDs must be unique within one response page\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "frozen-asset-access") {
    lines.push(
      "        if self.checksum_algorithm.is_some() != self.checksum_value.is_some() {",
      "            return Err(\"checksum_algorithm presence must match checksum_value\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "optional-checksum-pair") {
    lines.push(
      "        if self.checksum_algorithm.is_some() != self.checksum_value.is_some() {",
      "            return Err(\"checksum_algorithm presence must match checksum_value\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "asset-execution-policy") {
    lines.push(
      "        match &self.authority {",
      "            AssetExecutionAuthority::Legacy => {",
      "                if self.policy_id != \"legacy.v1\" {",
      "                    return Err(\"legacy policy_id must be legacy.v1\".to_string());",
      "                }",
      "            }",
      "            AssetExecutionAuthority::ResolverAuthoritative => {",
      "                if self.policy_id == \"legacy.v1\" {",
      "                    return Err(\"resolver policy_id must not be legacy.v1\".to_string());",
      "                }",
      "                if self.collection_profile_id.is_none() {",
      "                    return Err(\"resolver collection_profile_id is required\".to_string());",
      "                }",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "orbit-file-metadata") {
    lines.push(
      "        if self.file_name.contains('\\0')",
      "            || self.file_name.contains('/')",
      "            || self.file_name.contains('\\\\')",
      "        {",
      "            return Err(\"file_name must be a basename\".to_string());",
      "        }",
      "        if self.validity_stop_unix_seconds <= self.validity_start_unix_seconds {",
      "            return Err(\"validity stop must be after start\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "orbit-parse-request") {
    lines.push(
      "        for file_name in &self.file_names {",
      "            if file_name.is_empty()",
      "                || file_name.contains('\\0')",
      "                || file_name.contains('/')",
      "                || file_name.contains('\\\\')",
      "            {",
      "                return Err(\"file_names must contain basenames only\".to_string());",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "frozen-search-stats") {
    lines.push(
      "        if let Some((final_attempt, previous_attempts)) = self.source_attempts.split_last() {",
      "            if previous_attempts.iter().any(|attempt| {",
      "                attempt.outcome != FrozenSearchSourceAttemptOutcome::Fallback",
      "                    || attempt.fallback_reason.is_none()",
      "            }) {",
      "                return Err(",
      "                    \"source attempts before the final success must be bounded fallbacks\"",
      "                        .to_string(),",
      "                );",
      "            }",
      "            if final_attempt.outcome != FrozenSearchSourceAttemptOutcome::Succeeded",
      "                || final_attempt.fallback_reason.is_some()",
      "            {",
      "                return Err(",
      "                    \"source attempts must end in one success without a fallback reason\".to_string(),",
      "                );",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "frozen-search-batch") {
    lines.push(
      "        if let Some(aoi_geojson) = &self.aoi_geojson {",
      "            let geometry_type = aoi_geojson",
      "                .get(\"type\")",
      "                .and_then(Value::as_str)",
      "                .unwrap_or(\"\");",
      "            if !matches!(",
      "                geometry_type,",
      "                \"Polygon\" | \"MultiPolygon\" | \"Feature\" | \"FeatureCollection\" | \"GeometryCollection\"",
      "            ) {",
      "                return Err(\"aoi_geojson must contain real polygon topology\".to_string());",
      "            }",
      "        }",
      "        let candidate_ids: HashSet<&str> = self",
      "            .candidates",
      "            .iter()",
      "            .map(|item| item.item_id.as_str())",
      "            .collect();",
      "        if candidate_ids.len() != self.candidates.len() {",
      "            return Err(\"candidate item IDs must be unique\".to_string());",
      "        }",
      "        if self",
      "            .candidates",
      "            .iter()",
      "            .any(|item| item.provider_id != self.provider_id)",
      "        {",
      "            return Err(\"candidate provider IDs must match the batch provider\".to_string());",
      "        }",
      "        if let Some(search_stats) = &self.search_stats {",
      "            if let Some(final_attempt) = search_stats.source_attempts.last() {",
      "                if final_attempt.provider_id != self.provider_id {",
      "                    return Err(",
      "                        \"final source attempt provider must match the batch provider\".to_string(),",
      "                    );",
      "                }",
      "            }",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "frozen-task-snapshot") {
    lines.push(
      "        if let Some(aoi_geojson) = &self.aoi_geojson {",
      "            let geometry_type = aoi_geojson",
      "                .get(\"type\")",
      "                .and_then(Value::as_str)",
      "                .unwrap_or(\"\");",
      "            if !matches!(",
      "                geometry_type,",
      "                \"Polygon\" | \"MultiPolygon\" | \"Feature\" | \"FeatureCollection\" | \"GeometryCollection\"",
      "            ) {",
      "                return Err(\"aoi_geojson must contain real polygon topology\".to_string());",
      "            }",
      "        }",
      "        let candidate_ids: HashSet<&str> = self",
      "            .candidates",
      "            .iter()",
      "            .map(|item| item.item_id.as_str())",
      "            .collect();",
      "        if candidate_ids.len() != self.candidates.len() {",
      "            return Err(\"candidate item IDs must be unique\".to_string());",
      "        }",
      "        if self",
      "            .candidates",
      "            .iter()",
      "            .any(|item| item.provider_id != self.provider_id)",
      "        {",
      "            return Err(\"candidate provider IDs must match the task provider\".to_string());",
      "        }",
      "        let selected: HashSet<&str> = self",
      "            .initial_selected_item_ids",
      "            .iter()",
      "            .map(String::as_str)",
      "            .collect();",
      "        if selected.len() != self.initial_selected_item_ids.len()",
      "            || !selected.is_subset(&candidate_ids)",
      "        {",
      "            return Err(\"initial selected item IDs must be a unique candidate subset\".to_string());",
      "        }",
      "        if self.output_root.as_os_str().is_empty() {",
      "            return Err(\"output_root must not be empty\".to_string());",
      "        }",
    );
  }
  if (definition["x-insar-validation"] === "task-runtime-snapshot") {
    lines.push(
      "        let selected: HashSet<&str> = self.selected_item_ids.iter().map(String::as_str).collect();",
      "        let checked: HashSet<&str> = self.checked_item_ids.iter().map(String::as_str).collect();",
      "        if selected.len() != self.selected_item_ids.len()",
      "            || checked.len() != self.checked_item_ids.len()",
      "        {",
      "            return Err(\"selected and checked item IDs must be unique\".to_string());",
      "        }",
      "        let lifecycle_lists = [",
      "            &self.active_item_ids,",
      "            &self.paused_item_ids,",
      "            &self.queued_item_ids,",
      "            &self.completed_item_ids,",
      "            &self.failed_item_ids,",
      "            &self.skipped_item_ids,",
      "            &self.cancelled_item_ids,",
      "        ];",
      "        let lifecycle: Vec<HashSet<&str>> = lifecycle_lists",
      "            .iter()",
      "            .map(|values| values.iter().map(String::as_str).collect())",
      "            .collect();",
      "        if lifecycle",
      "            .iter()",
      "            .zip(lifecycle_lists)",
      "            .any(|(values, raw)| values.len() != raw.len())",
      "        {",
      "            return Err(\"task lifecycle item IDs must be unique\".to_string());",
      "        }",
      "        if lifecycle.iter().any(|values| !values.is_subset(&selected)) {",
      "            return Err(\"task lifecycle item IDs must be selected\".to_string());",
      "        }",
      "        for (index, left) in lifecycle.iter().enumerate() {",
      "            if lifecycle[index + 1..]",
      "                .iter()",
      "                .any(|right| !left.is_disjoint(right))",
      "            {",
      "                return Err(\"task lifecycle sets must be mutually exclusive\".to_string());",
      "            }",
      "        }",
      "        if self.active_item_ids.len() > self.concurrency as usize {",
      "            return Err(\"active item count exceeds concurrency\".to_string());",
      "        }",
    );
  }
  lines.push("        Ok(())", "    }", "}");
  return lines;
}

function renderRust(schema) {
  const protocol = protocolVersions(schema);
  const lines = [
    "// Generated by scripts/generate_v3_contracts.mjs.",
    "// Source: contracts/v3/provider-contract.schema.json. DO NOT EDIT.",
    "use serde::{Deserialize, Deserializer, Serialize};",
    "use serde_json::{Map, Value};",
    "use std::collections::HashSet;",
    "use std::path::PathBuf;",
    "use url::Url;",
    "",
    `pub const V3_CONTRACT_VERSION: &str = ${JSON.stringify(contractVersion(schema))};`,
    `pub const V3_PROTOCOL_MAJOR: u64 = ${protocol.major};`,
    `pub const V3_PROTOCOL_MINOR: u64 = ${protocol.minor};`,
    `pub const V3_PROTOCOL_MIN_MINOR: u64 = ${protocol.minimumMinor};`,
    "",
    "fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>",
    "where",
    "    D: Deserializer<'de>,",
    "    T: Deserialize<'de>,",
    "{",
    "    Option::<T>::deserialize(deserializer)",
    "}",
  ];
  const defaultFunctions = [];
  const hasSensitivePolicy = Object.values(schema.$defs).some((definition) =>
    Object.values(definition.properties ?? {}).some(
      (property) => property["x-insar-sensitive-key-policy"] === true,
    ));
  if (hasSensitivePolicy) {
    lines.push(
      "",
      "fn validate_debug_value(value: &Value) -> Result<(), String> {",
      "    match value {",
      "        Value::Object(map) => validate_debug_context(map),",
      "        Value::Array(items) => {",
      "            for item in items {",
      "                validate_debug_value(item)?;",
      "            }",
      "            Ok(())",
      "        }",
      "        _ => Ok(()),",
      "    }",
      "}",
      "",
      "fn validate_debug_context(context: &Map<String, Value>) -> Result<(), String> {",
      "    const SENSITIVE: &[&str] = &[",
      "        \"password\",",
      "        \"token\",",
      "        \"cookie\",",
      "        \"authorization\",",
      "        \"secret\",",
      "        \"netrc\",",
      "        \".netrc\",",
      "        \"environment\",",
      "        \"env\",",
      "    ];",
      "    for (key, value) in context {",
      "        let normalized = key.to_ascii_lowercase();",
      "        if SENSITIVE.iter().any(|item| normalized == *item)",
      "            || normalized.ends_with(\"_token\")",
      "            || normalized.ends_with(\"_password\")",
      "        {",
      "            return Err(format!(\"sensitive debug-context key is forbidden: {key}\"));",
      "        }",
      "        validate_debug_value(value)?;",
      "    }",
      "    Ok(())",
      "}",
    );
  }
  lines.push(
    "",
    "fn validate_catalogue_position(value: &Value, field_name: &str) -> Result<(f64, f64), String> {",
    "    let position = value",
    "        .as_array()",
    "        .ok_or_else(|| format!(\"{field_name} positions must contain longitude and latitude\"))?;",
    "    if position.len() < 2 {",
    "        return Err(format!(",
    "            \"{field_name} positions must contain longitude and latitude\"",
    "        ));",
    "    }",
    "    for coordinate in position {",
    "        if !coordinate.as_f64().is_some_and(f64::is_finite) {",
    "            return Err(format!(\"{field_name} positions must be finite numbers\"));",
    "        }",
    "    }",
    "    let longitude = position[0].as_f64().expect(\"validated finite number\");",
    "    let latitude = position[1].as_f64().expect(\"validated finite number\");",
    "    if !(-180.0..=180.0).contains(&longitude) || !(-90.0..=90.0).contains(&latitude) {",
    "        return Err(format!(",
    "            \"{field_name} must use EPSG:4326 longitude/latitude coordinates\"",
    "        ));",
    "    }",
    "    Ok((longitude, latitude))",
    "}",
    "",
    "fn validate_catalogue_linear_ring(value: &Value, field_name: &str) -> Result<(), String> {",
    "    let ring = value.as_array().ok_or_else(|| {",
    "        format!(\"{field_name} polygon rings must contain at least four positions\")",
    "    })?;",
    "    if ring.len() < 4 {",
    "        return Err(format!(",
    "            \"{field_name} polygon rings must contain at least four positions\"",
    "        ));",
    "    }",
    "    let first = validate_catalogue_position(&ring[0], field_name)?;",
    "    let mut last = first;",
    "    let mut distinct: Vec<(f64, f64)> = Vec::with_capacity(3);",
    "    distinct.push(first);",
    "    for (index, raw_position) in ring.iter().enumerate().skip(1) {",
    "        let pos = validate_catalogue_position(raw_position, field_name)?;",
    "        last = pos;",
    "        if index + 1 < ring.len() && distinct.len() < 3 && !distinct.iter().any(|known| known == &pos) {",
    "            distinct.push(pos);",
    "        }",
    "    }",
    "    if first != last {",
    "        return Err(format!(\"{field_name} polygon rings must be closed\"));",
    "    }",
    "    if distinct.len() < 3 {",
    "        return Err(format!(",
    "            \"{field_name} polygon rings must contain three distinct positions\"",
    "        ));",
    "    }",
    "    Ok(())",
    "}",
    "",
    "fn validate_catalogue_polygon_coordinates(value: &Value, field_name: &str) -> Result<(), String> {",
    "    let rings = value",
    "        .as_array()",
    "        .filter(|rings| !rings.is_empty())",
    "        .ok_or_else(|| {",
    "            format!(\"{field_name} Polygon coordinates must contain at least one ring\")",
    "        })?;",
    "    for ring in rings {",
    "        validate_catalogue_linear_ring(ring, field_name)?;",
    "    }",
    "    Ok(())",
    "}",
    "",
    "fn validate_catalogue_polygonal_geometry(value: &Value, field_name: &str) -> Result<(), String> {",
    "    let object = value",
    "        .as_object()",
    "        .ok_or_else(|| format!(\"{field_name} must be a GeoJSON object\"))?;",
    "    validate_catalogue_polygonal_map(object, field_name)",
    "}",
    "",
    "fn validate_catalogue_polygonal_map(object: &Map<String, Value>, field_name: &str) -> Result<(), String> {",
    "    validate_debug_context(object)?;",
    "    if object.contains_key(\"crs\") {",
    "        return Err(format!(",
    "            \"{field_name} must not declare a GeoJSON crs member\"",
    "        ));",
    "    }",
    "    match object.get(\"type\").and_then(Value::as_str) {",
    "        Some(\"Polygon\") => validate_catalogue_polygon_coordinates(",
    "            object.get(\"coordinates\").unwrap_or(&Value::Null),",
    "            field_name,",
    "        ),",
    "        Some(\"MultiPolygon\") => {",
    "            let polygons = object",
    "                .get(\"coordinates\")",
    "                .and_then(Value::as_array)",
    "                .filter(|polygons| !polygons.is_empty())",
    "                .ok_or_else(|| {",
    "                    format!(",
    "                        \"{field_name} MultiPolygon coordinates must contain at least one polygon\"",
    "                    )",
    "                })?;",
    "            for polygon in polygons {",
    "                validate_catalogue_polygon_coordinates(polygon, field_name)?;",
    "            }",
    "            Ok(())",
    "        }",
    "        Some(\"Feature\") => validate_catalogue_polygonal_geometry(",
    "            object.get(\"geometry\").unwrap_or(&Value::Null),",
    "            field_name,",
    "        ),",
    "        Some(\"FeatureCollection\") => {",
    "            let features = object",
    "                .get(\"features\")",
    "                .and_then(Value::as_array)",
    "                .filter(|features| !features.is_empty())",
    "                .ok_or_else(|| {",
    "                    format!(\"{field_name} FeatureCollection must contain polygonal features\")",
    "                })?;",
    "            for feature in features {",
    "                if feature.get(\"type\").and_then(Value::as_str) != Some(\"Feature\") {",
    "                    return Err(format!(",
    "                        \"{field_name} FeatureCollection must contain GeoJSON Features\"",
    "                    ));",
    "                }",
    "                validate_catalogue_polygonal_geometry(feature, field_name)?;",
    "            }",
    "            Ok(())",
    "        }",
    "        _ => Err(format!(",
    "            \"{field_name} must contain Polygon or MultiPolygon topology\"",
    "        )),",
    "    }",
    "}",
    "",
    "fn validate_catalogue_polygonal_geojson(",
    "    value: &Map<String, Value>,",
    "    field_name: &str,",
    ") -> Result<(), String> {",
    "    validate_catalogue_polygonal_map(value, field_name)",
    "}",
    "",
    "fn validate_oauth_bearer_https_href(stable_href: &str) -> Result<(), String> {",
    "    if stable_href",
    "        .chars()",
    "        .any(|character| character.is_control() || character.is_whitespace() || character == '\\\\')",
    "    {",
    "        return Err(",
    "            \"oauth_bearer_https stable_href must not contain control, whitespace, or backslash characters\"",
    "                .to_string(),",
    "        );",
    "    }",
    "    let authority = stable_href",
    "        .strip_prefix(\"https://\")",
    "        .ok_or_else(|| \"oauth_bearer_https stable_href must be an absolute HTTPS URL\".to_string())?",
    "        .split('/')",
    "        .next()",
    "        .unwrap_or(\"\");",
    "    if authority.is_empty() || authority.contains('@') {",
    "        return Err(",
    "            \"oauth_bearer_https stable_href must have a non-empty host without userinfo\"",
    "                .to_string(),",
    "        );",
    "    }",
    "    if authority.ends_with(':') {",
    "        return Err(\"oauth_bearer_https stable_href must not use an empty port\".to_string());",
    "    }",
    "    let parsed = Url::parse(stable_href)",
    "        .map_err(|_| \"oauth_bearer_https stable_href must be an absolute HTTPS URL\".to_string())?;",
    "    if parsed.scheme() != \"https\" || parsed.host_str().is_none() {",
    "        return Err(\"oauth_bearer_https stable_href must be an absolute HTTPS URL\".to_string());",
    "    }",
    "    if !parsed.username().is_empty() || parsed.password().is_some() {",
    "        return Err(\"oauth_bearer_https stable_href must not contain userinfo\".to_string());",
    "    }",
    "    if parsed.query().is_some() || parsed.fragment().is_some() {",
    "        return Err(",
    "            \"oauth_bearer_https stable_href must not contain query or fragment\".to_string(),",
    "        );",
    "    }",
    "    if parsed.port_or_known_default() != Some(443) {",
    "        return Err(\"oauth_bearer_https stable_href must use port 443\".to_string());",
    "    }",
    "    Ok(())",
    "}",
  );

  for (const [name, definition] of Object.entries(schema.$defs)) {
    lines.push("");
    if (definition.enum) {
      lines.push("#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]");
      lines.push(`pub enum ${name} {`);
      for (const value of definition.enum) {
        lines.push(`    #[serde(rename = ${JSON.stringify(value)})]`);
        lines.push(`    ${pascalCase(value)},`);
      }
      lines.push("}");
      continue;
    }
    lines.push("#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]");
    lines.push("#[serde(deny_unknown_fields)]");
    const properties = Object.entries(definition.properties ?? {});
    const required = new Set(definition.required ?? []);
    if (properties.length === 0) {
      lines.push(`pub struct ${name} {}`);
    } else {
      lines.push(`pub struct ${name} {`);
      for (const [fieldName, fieldSchema] of properties) {
        if (
          required.has(fieldName)
          && isNullable(fieldSchema)
          && fieldSchema.default === undefined
          && fieldSchema["x-insar-strict"] === true
        ) {
          lines.push("    #[serde(deserialize_with = \"deserialize_required_nullable\")]");
        }
        lines.push(...rustDefaultAttribute(name, fieldName, fieldSchema, defaultFunctions));
        lines.push(`    pub ${rustIdentifier(fieldName)}: ${rustType(fieldSchema)},`);
      }
      lines.push("}");
    }
    lines.push(...rustValidationLines(name, definition, schema));
  }
  if (defaultFunctions.length) {
    const constantIndex = lines.findIndex((line) => line.startsWith("pub const V3_CONTRACT_VERSION"));
    lines.splice(constantIndex, 0, ...defaultFunctions, "");
  }
  return `${lines.join("\n")}\n`;
}

function typeScriptType(node) {
  if (node.const !== undefined) {
    const rendered = JSON.stringify(node.const);
    return isNullable(node) ? `${rendered} | null` : rendered;
  }
  if (node.enum !== undefined) {
    const values = node.enum.filter((value) => value !== null);
    const rendered = values.length ? values.map((value) => JSON.stringify(value)).join(" | ") : "null";
    return node.enum.includes(null) && values.length ? `${rendered} | null` : rendered;
  }
  if (node.$ref) {
    const rendered = referenceName(node);
    return isNullable(node) ? `${rendered} | null` : rendered;
  }
  let rendered;
  switch (nonNullType(node)) {
    case "string":
      rendered = "string";
      break;
    case "integer":
    case "number":
      rendered = "number";
      break;
    case "boolean":
      rendered = "boolean";
      break;
    case "array": {
      const itemType = typeScriptType(node.items);
      const arrayItemType = itemType.includes(" | ") ? `(${itemType})` : itemType;
      if (node.minItems !== undefined && node.minItems > 0) {
        const requiredItems = Array.from({ length: node.minItems }, () => arrayItemType).join(", ");
        rendered = `[${requiredItems}, ...${arrayItemType}[]]`;
      } else {
        rendered = `${arrayItemType}[]`;
      }
      break;
    }
    case "object":
      if (node["x-insar-opaque-object"] === true) {
        rendered = "Record<string, unknown>";
        break;
      }
      throw new Error("inline typed objects are not supported in TypeScript fields");
    default:
      throw new Error("unsupported TypeScript field type");
  }
  return isNullable(node) ? `${rendered} | null` : rendered;
}

function renderTypeScript(schema) {
  const protocol = protocolVersions(schema);
  const lines = [
    "// Generated by scripts/generate_v3_contracts.mjs.",
    "// Source: contracts/v3/provider-contract.schema.json. DO NOT EDIT.",
    `export const V3_CONTRACT_VERSION = ${JSON.stringify(contractVersion(schema))} as const;`,
    `export const V3_PROTOCOL_MAJOR = ${protocol.major} as const;`,
    `export const V3_PROTOCOL_MINOR = ${protocol.minor} as const;`,
    `export const V3_PROTOCOL_MIN_MINOR = ${protocol.minimumMinor} as const;`,
  ];
  for (const [name, definition] of Object.entries(schema.$defs)) {
    lines.push("");
    if (definition.enum) {
      lines.push(`export type ${name} = ${definition.enum.map((value) => JSON.stringify(value)).join(" | ")};`);
      continue;
    }
    lines.push(`export type ${name} = {`);
    const required = new Set(definition.required ?? []);
    for (const [fieldName, fieldSchema] of Object.entries(definition.properties ?? {})) {
      lines.push(`  ${fieldName}${required.has(fieldName) ? "" : "?"}: ${typeScriptType(fieldSchema)};`);
    }
    lines.push("};");
  }
  return `${lines.join("\n")}\n`;
}

export function parseArgs(argv) {
  const options = {
    check: false,
    schema: "contracts/v3/provider-contract.schema.json",
    outRoot: ".",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--check") {
      options.check = true;
    } else if (argument === "--schema" || argument === "--out-root") {
      const value = argv[index + 1];
      if (!value) throw new Error(`${argument} requires a path`);
      if (argument === "--schema") options.schema = value;
      else options.outRoot = value;
      index += 1;
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return options;
}

export function generateBindings(schema) {
  validateSchema(schema);
  return {
    python: renderPython(schema),
    rust: renderRust(schema),
    typescript: renderTypeScript(schema),
  };
}

export async function writeBindings(bindings, outRoot) {
  for (const [language, relativePath] of Object.entries(OUTPUTS)) {
    const outputPath = path.join(outRoot, relativePath);
    await mkdir(path.dirname(outputPath), { recursive: true });
    await writeFile(outputPath, bindings[language], "utf8");
  }
}

export async function checkBindings(bindings, outRoot) {
  const stale = [];
  for (const [language, relativePath] of Object.entries(OUTPUTS)) {
    let current;
    try {
      current = await readFile(path.join(outRoot, relativePath), "utf8");
    } catch {
      current = undefined;
    }
    if (current !== bindings[language]) stale.push(relativePath.replaceAll("\\", "/"));
  }
  if (stale.length) {
    throw new Error(stale.map((file) => `stale generated file: ${file}`).join("\n"));
  }
}

// The catalog is a separate versioned contract; generate both language bindings
// from its entries, never maintain a parallel handwritten tool allowlist.
export function generateAgentBindings(catalog, schema) {
  if (schema.properties?.tools?.items?.$ref !== "#/$defs/tool" || !schema.$defs?.tool) {
    throw new Error("invalid agent catalog schema");
  }
  if (catalog.schema_version !== 1 || catalog.catalog_id !== "eostage.agent-tools.v1"
    || !Array.isArray(catalog.tools)) throw new Error("invalid agent catalog header");
  const required = schema.$defs.tool.required;
  const enums = schema.$defs.tool.properties;
  const seen = new Set();
  for (const tool of catalog.tools) {
    for (const key of required) if (!Object.hasOwn(tool, key)) throw new Error(`missing agent tool ${key}`);
    if (seen.has(tool.tool_id)) throw new Error(`duplicate agent tool: ${tool.tool_id}`);
    seen.add(tool.tool_id);
    for (const key of ["job", "effect", "confirmation"]) {
      const allowed = key === "job" ? enums.job.enum : enums[key].enum;
      if (!allowed.includes(tool[key])) throw new Error(`invalid agent tool ${key}`);
    }
    if (!tool.evidence.length) throw new Error(`missing agent evidence: ${tool.tool_id}`);
  }
  const ids = catalog.tools.map(({tool_id}) => tool_id);
  return {
    rust: "// Generated by scripts/generate_v3_contracts.mjs. DO NOT EDIT.\n"
      + `pub const TOOL_IDS: &[&str] = &[${ids.map((id) => JSON.stringify(id)).join(", ")}];\n`,
    typescript: "// Generated by scripts/generate_v3_contracts.mjs. DO NOT EDIT.\n"
      + `export const AGENT_TOOL_IDS = ${JSON.stringify(ids)} as const;\n`
      + "export type AgentToolId = typeof AGENT_TOOL_IDS[number];\n",
  };
}

export async function main(argv = process.argv.slice(2)) {
  if (argv.includes("--help")) {
    process.stdout.write(
      "Usage: node scripts/generate_v3_contracts.mjs "
        + "[--check] [--schema PATH] [--out-root PATH]\n",
    );
    return 0;
  }

  const options = parseArgs(argv);
  const schemaText = await readFile(options.schema, "utf8");
  const schema = JSON.parse(schemaText);
  const bindings = generateBindings(schema);
  if (options.check) await checkBindings(bindings, options.outRoot);
  else await writeBindings(bindings, options.outRoot);
  // Explicit --schema is used by generator unit tests for isolated fixtures.
  if (options.schema === "contracts/v3/provider-contract.schema.json") {
    const catalog = JSON.parse(await readFile("contracts/v3/agent-tool-catalog.v1.json", "utf8"));
    const catalogSchema = JSON.parse(await readFile("contracts/v3/agent-tool-catalog.schema.json", "utf8"));
    const agentBindings = generateAgentBindings(catalog, catalogSchema);
    for (const [language, relativePath] of Object.entries(AGENT_OUTPUTS)) {
      const outputPath = path.join(options.outRoot, relativePath);
      if (options.check) {
        const current = await readFile(outputPath, "utf8").catch(() => undefined);
        if (current !== agentBindings[language]) throw new Error(`stale generated file: ${relativePath}`);
      } else {
        await mkdir(path.dirname(outputPath), { recursive: true });
        await writeFile(outputPath, agentBindings[language], "utf8");
      }
    }
  }
  return 0;
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  main().then(
    (exitCode) => {
      process.exitCode = exitCode;
    },
    (error) => {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
      process.exitCode = 1;
    },
  );
}
