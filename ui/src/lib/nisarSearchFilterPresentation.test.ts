import assert from "node:assert/strict";
import test from "node:test";
import { nisarSearchFilterOptionLabel as label } from "./nisarSearchFilterPresentation.ts";

test("NISAR L-band filter labels describe metadata without changing CMR values", () => {
  assert.match(label("frame_coverage", "FULL"), /完整帧.*FULL/);
  assert.match(label("frequency_a_polarization", "HH+HV"), /HH\+HV.*双极化/);
  assert.match(label("frequency_b_polarization", "VV"), /VV.*单极化/);
  assert.equal(label("range_bandwidth", "20+5"), "20+5 MHz");
  assert.match(label("flight_direction", "ASCENDING"), /升轨/);
  assert.match(label("look_direction", "RIGHT"), /右视/);
});

test("NISAR S-band provider polarization codes retain their scientific channel meaning", () => {
  for (const [code, channels] of [
    ["SH", "HH"], ["SV", "VV"], ["DH", "HH/HV"], ["DV", "VV/VH"],
  ]) {
    assert.match(label("polarization", code), new RegExp(channels));
  }
  assert.match(label("polarization", "CP"), /紧凑极化.*圆极化发射.*接收通道依影像/);
  assert.equal(label("mode", "W"), "W");
  assert.equal(label("acquisition_mode", "J"), "J");
  assert.equal(label("acquisition_mode", "W"), "W");
  assert.equal(label("software_version", "1.4.00"), "1.4.00");
  assert.equal(label("polarization", "UNREVIEWED"), "UNREVIEWED");
  assert.doesNotMatch(
    [
      label("range_bandwidth", "20+5"),
      label("mode", "W"),
      label("acquisition_mode", "J"),
      label("polarization", "UNREVIEWED"),
    ].join(" "),
    /目录原值|提供方(?:目录)?编码/,
  );
});
