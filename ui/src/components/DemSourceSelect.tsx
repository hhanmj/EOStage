import { DEM_DATASET_OPTIONS } from "@/lib/demSources";

type DemSourceSelectProps = {
  dataset: string;
  onChange: (dataset: string) => void;
  className?: string;
};

/** Product/resolution is the user choice. Provider routing stays internal. */
export function DemSourceSelect({ dataset, onChange, className }: DemSourceSelectProps) {
  return (
    <select
      aria-label="DEM 产品与分辨率"
      value={dataset}
      onChange={(event) => onChange(event.target.value)}
      className={className}
    >
      {DEM_DATASET_OPTIONS.map((option) => (
        <option key={option.value} value={option.value} disabled={!option.enabled}>
          {option.label}{option.hint ? ` · ${option.hint}` : ""}
        </option>
      ))}
    </select>
  );
}
