import "./setup-dom";
import {
  render as rtlRender,
  cleanup,
  fireEvent,
  type RenderOptions,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { testDom } from "./setup-dom";

export function setupUser() {
  return userEvent.setup({ document: testDom.document });
}

export function renderComponent(ui: ReactElement, options?: Omit<RenderOptions, "queries">) {
  return rtlRender(ui, { ...options });
}

export { cleanup, fireEvent, rtlRender as render };
