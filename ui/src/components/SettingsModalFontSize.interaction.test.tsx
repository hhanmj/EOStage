import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import {
  SHELL_FONT_SIZE_STORAGE_KEY,
  SettingsModal,
  restoreGlobalFontSizeFromStorage,
} from "./SettingsModal";
import { I18nProvider } from "@/i18n";

test.afterEach(() => {
  cleanup();
  window.localStorage.removeItem(SHELL_FONT_SIZE_STORAGE_KEY);
});

test("SettingsModal font size numeric input commits on blur and enter, and reverts invalid values", () => {
  let changedSize: number | null = null;
  const view = render(
    <I18nProvider>
      <SettingsModal
        open={true}
        onClose={() => {}}
        initialCategory="appearance"
        fontSize={14}
        onFontSizeChange={(px) => {
          changedSize = px;
        }}
      />
    </I18nProvider>,
  );

  const input = view.getByRole("spinbutton", { name: "字号" }) as HTMLInputElement;
  assert.equal(input.value, "14");

  // 1. 合法值失焦生效
  fireEvent.change(input, { target: { value: "18" } });
  assert.equal(input.value, "18");
  fireEvent.blur(input);
  assert.equal(changedSize, 18);
  assert.equal(input.value, "18");

  // 2. 合法值回车生效
  fireEvent.change(input, { target: { value: "22" } });
  fireEvent.keyDown(input, { key: "Enter" });
  assert.equal(changedSize, 22);
  assert.equal(input.value, "22");

  // 3. 超上限非法值失焦回退 (25 > 22 -> 回退为 14)
  fireEvent.change(input, { target: { value: "25" } });
  fireEvent.blur(input);
  assert.equal(input.value, "14");

  // 4. 低下限非法值失焦回退 (11 < 12 -> 回退为 14)
  fireEvent.change(input, { target: { value: "11" } });
  fireEvent.blur(input);
  assert.equal(input.value, "14");

  // 5. 非数字非法值回退
  fireEvent.change(input, { target: { value: "abc" } });
  fireEvent.blur(input);
  assert.equal(input.value, "14");
});

test("SettingsModal font size presets (小/标准/大/特大) apply immediate global scale and persist across reload", () => {
  function StatefulHarness() {
    const [size, setSize] = React.useState(14);
    return (
      <div className="sp-modern">
        <I18nProvider>
          <SettingsModal
            open={true}
            onClose={() => {}}
            initialCategory="appearance"
            fontSize={size}
            onFontSizeChange={setSize}
          />
        </I18nProvider>
      </div>
    );
  }

  const view = render(<StatefulHarness />);
  const root = document.documentElement;
  const spModern = document.querySelector(".sp-modern") as HTMLElement;

  fireEvent.click(view.getByRole("button", { name: "特大" }));
  assert.equal(root.style.getPropertyValue("--sp-font-size"), "16px");
  assert.equal(root.style.getPropertyValue("--sp-font-scale"), "1.1429");
  assert.equal(spModern.style.getPropertyValue("--sp-font-scale"), "1.1429");
  assert.equal(window.localStorage.getItem(SHELL_FONT_SIZE_STORAGE_KEY), "16");

  fireEvent.click(view.getByRole("button", { name: "小" }));
  assert.equal(root.style.getPropertyValue("--sp-font-size"), "13px");
  assert.equal(root.style.getPropertyValue("--sp-font-scale"), "0.9286");
  assert.equal(window.localStorage.getItem(SHELL_FONT_SIZE_STORAGE_KEY), "13");

  // Simulate reload restoration from localStorage
  root.style.removeProperty("--sp-font-size");
  root.style.removeProperty("--sp-font-scale");
  const restored = restoreGlobalFontSizeFromStorage();
  assert.equal(restored, 13);
  assert.equal(root.style.getPropertyValue("--sp-font-size"), "13px");
  assert.equal(root.style.getPropertyValue("--sp-font-scale"), "0.9286");
});
