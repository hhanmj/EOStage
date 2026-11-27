import type { ReactNode } from "react";

import { FilterDateRange } from "@/components/FilterDateRange";
import { SarFilterGroup } from "@/components/SarFilterGroup";
import { Input } from "@/components/ui/input";
import {
  conditionFilters,
  draftDateRange,
  draftSelection,
  draftText,
  facetFilters,
  multiEnumMenuValues,
  updateMultiEnumSelection,
  type FacetSearchFilter,
  type SearchFilterDraft,
  type SearchFilterDraftValue,
  type SearchFilterSchema,
} from "@/lib/searchFilterSchema";

/**
 * Issue #3 R2: generic fields rendered from a reviewed search filter schema.
 * The component owns no state; the caller keeps one draft per dataset.
 * Facets reuse the caller's shared multi-select menu so SAR and optical
 * filters keep one interaction.
 */
type SchemaFieldsProps = {
  schema: SearchFilterSchema;
  draft: SearchFilterDraft;
  onDraftChange: (filterId: string, value: SearchFilterDraftValue) => void;
  disabled: boolean;
  /** Prefix for accessible names, normally the dataset label. */
  labelPrefix: string;
};

export type SchemaMultiSelectRenderer = (args: {
  filter: FacetSearchFilter;
  ariaLabel: string;
  /** Empty means "全部" in the shared menu. */
  values: string[];
  onChange: (next: string[]) => void;
  disabled: boolean;
}) => ReactNode;

export function SchemaFacetFields({
  schema,
  draft,
  onDraftChange,
  disabled,
  labelPrefix,
  renderMultiSelect,
}: SchemaFieldsProps & { renderMultiSelect: SchemaMultiSelectRenderer }) {
  return (
    <>
      {facetFilters(schema).map((filter) => {
        const multi = filter.control === "multi_enum" ? filter : null;
        return (
          <fieldset
            key={filter.filter_id}
            className="min-w-0"
            disabled={disabled}
            data-search-filter-group={filter.filter_id}
          >
            <legend className="mb-1 text-[11px] font-medium text-muted-foreground">{filter.label}</legend>
            {renderMultiSelect({
              filter,
              ariaLabel: `${labelPrefix} ${filter.label}`,
              values: multi ? multiEnumMenuValues(multi, draftSelection(draft, filter.filter_id)) : [],
              onChange: multi
                ? (next) => onDraftChange(filter.filter_id, updateMultiEnumSelection(multi, next))
                : () => undefined,
              disabled,
            })}
          </fieldset>
        );
      })}
    </>
  );
}

export function SchemaConditionFields({
  schema,
  draft,
  onDraftChange,
  disabled,
  labelPrefix,
  dateHelpSuffix,
}: SchemaFieldsProps & { dateHelpSuffix?: string }) {
  return (
    <>
      {conditionFilters(schema).map((filter) => {
        if (filter.control === "date_range") {
          const range = draftDateRange(draft, filter.filter_id);
          const help = [filter.help, dateHelpSuffix].filter(Boolean).join("；");
          return (
            <SarFilterGroup key={filter.filter_id} title={filter.label} className="grid-cols-1">
              <FilterDateRange
                start={range.start}
                end={range.end}
                disabled={disabled}
                onChange={({ start, end }) => onDraftChange(filter.filter_id, { start, end })}
              />
              {help && <span className="text-[10px] leading-4 text-muted-foreground">{help}</span>}
            </SarFilterGroup>
          );
        }
        return (
          <label key={filter.filter_id} className="block min-w-0" data-search-filter-field={filter.filter_id}>
            <span className="mb-0.5 block text-[11px] font-medium text-muted-foreground">
              {filter.unit ? `${filter.label}（${filter.unit}）` : filter.label}
            </span>
            <Input
              value={draftText(draft, filter.filter_id)}
              onChange={(event) => onDraftChange(filter.filter_id, event.currentTarget.value)}
              placeholder={filter.placeholder}
              aria-label={`${labelPrefix} ${filter.label}`}
              inputMode="decimal"
              disabled={disabled}
            />
          </label>
        );
      })}
    </>
  );
}
