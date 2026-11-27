import assert from "node:assert/strict";
import test from "node:test";

import { approvedExternalUrl, desktopExternalUrl } from "./externalNavigation.ts";

test("packaged desktop links open http resources externally", () => {
  assert.equal(
    desktopExternalUrl(
      "https://leafletjs.com/",
      "file:///E:/InSAR%20Studio/insar_prep/desktop/web/index.html",
    ),
    "https://leafletjs.com/",
  );
});

test("same-origin app navigation and non-http protocols stay inside the app policy", () => {
  assert.equal(
    desktopExternalUrl("/assets/help.html", "http://localhost:5173/index.html"),
    null,
  );
  assert.equal(desktopExternalUrl("mailto:test@example.com", "file:///app/index.html"), null);
  assert.equal(desktopExternalUrl("javascript:alert(1)", "file:///app/index.html"), null);
  assert.equal(approvedExternalUrl("file:///E:/secret.txt"), null);
  assert.equal(approvedExternalUrl("data:text/plain,blocked"), null);
  assert.equal(approvedExternalUrl("https://leafletjs.com/"), "https://leafletjs.com/");
});
