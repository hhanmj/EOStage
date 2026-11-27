/** Display only values supplied by the admitted NISAR source capability.
 * These labels explain closed provider codes; they never create query choices.
 */
export function nisarSearchFilterOptionLabel(key: string, value: string): string {
  if (key === "frame_coverage") {
    if (value === "FULL") return "完整帧（FULL）";
    if (value === "PARTIAL") return "部分帧（PARTIAL）";
  }
  if (key === "flight_direction") {
    if (value === "ASCENDING") return "升轨（ASCENDING）";
    if (value === "DESCENDING") return "降轨（DESCENDING）";
  }
  if (key === "look_direction") {
    if (value === "LEFT") return "左视（LEFT）";
    if (value === "RIGHT") return "右视（RIGHT）";
  }
  if (key === "frequency_a_polarization" || key === "frequency_b_polarization") {
    const channels = value.split("+");
    const mode = channels.length === 1 ? "单极化" : channels.length === 2 ? "双极化" : channels.length === 4 ? "四极化" : null;
    return mode ? `${value}（${mode}）` : value;
  }
  if (key === "range_bandwidth") return `${value} MHz`;
  if (key === "polarization") {
    const known: Record<string, string> = {
      SH: "单极化 HH",
      SV: "单极化 VV",
      DH: "双极化 HH/HV",
      DV: "双极化 VV/VH",
      CP: "紧凑极化（圆极化发射；接收通道依影像）",
    };
    return known[value] ? `${value}（${known[value]}）` : value;
  }
  if (key === "mode" || key === "acquisition_mode") return value;
  return value;
}
