import { Input } from "./ui/input";
import type { FilterDescriptor } from "../lib/generated/v3-contracts";
import type { SarSearchFormRange } from "../lib/sarSearchForm";

type SarRangeKind = "track" | "frame";
type SarRangeEndpoint = "start" | "end";

type SarSearchFilterInputsProps = {
  trackDescriptor: FilterDescriptor | null;
  frameDescriptor: FilterDescriptor | null;
  trackRange: SarSearchFormRange;
  frameRange: SarSearchFormRange;
  onRangeChange: (kind: SarRangeKind, endpoint: SarRangeEndpoint, value: string) => void;
};

function rangeInputs(
  kind: SarRangeKind,
  label: "Path" | "Frame",
  descriptor: FilterDescriptor | null,
  range: SarSearchFormRange,
  onRangeChange: SarSearchFilterInputsProps["onRangeChange"],
) {
  if (!descriptor?.supported) return null;
  return (["start", "end"] as const).map((endpoint) => {
    const endpointLabel = endpoint === "start" ? "起" : "止";
    return (
      <label key={`${kind}-${endpoint}`} className="min-w-0">
        <Input
          type="text"
          value={range[endpoint]}
          onChange={(event) => onRangeChange(kind, endpoint, event.currentTarget.value)}
          className="h-8 px-2 text-xs placeholder:text-[10px]"
          aria-label={`${descriptor?.label || label} ${endpointLabel}`}
          title={`${descriptor.label || label} ${endpoint === "start" ? "起始值" : "结束值"}`}
          inputMode={descriptor?.value_type === "integer" ? "numeric" : "text"}
          pattern={descriptor?.pattern ?? undefined}
          placeholder={`${label} ${endpointLabel}`}
          autoComplete="off"
          spellCheck={false}
          data-testid={`sar-${kind === "track" ? "path" : "frame"}-${endpoint}`}
        />
      </label>
    );
  });
}

export function SarSearchFilterInputs({
  trackDescriptor,
  frameDescriptor,
  trackRange,
  frameRange,
  onRangeChange,
}: SarSearchFilterInputsProps) {
  return (
    <>
      {rangeInputs("track", "Path", trackDescriptor, trackRange, onRangeChange)}
      {rangeInputs("frame", "Frame", frameDescriptor, frameRange, onRangeChange)}
    </>
  );
}

type SarResultLimitInputProps = {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
};

export function SarResultLimitInput({ value, onChange, disabled = false }: SarResultLimitInputProps) {
  return (
    <label className="min-w-0">
      <span className="flex items-center gap-2">
        <span className="min-w-0 flex-1 text-[10px] font-medium text-muted-foreground">结果数量</span>
        <Input
          type="text"
          value={value}
          onChange={(event) => onChange(event.currentTarget.value)}
          className="h-8 w-28 px-2 text-xs placeholder:text-[10px]"
          placeholder="留空"
          inputMode="numeric"
          aria-label="结果数量"
          title="留空请求 250 景；填写 1–10000 的整数可主动请求更多，250 仅为单页容量。"
          autoComplete="off"
          spellCheck={false}
          disabled={disabled}
          data-testid="sar-max-results"
        />
      </span>
      <span className="mt-0.5 block text-[10px] leading-3 text-muted-foreground">留空默认请求 250 景；显式填写可超过 250，单页最多显示 250 景</span>
    </label>
  );
}
