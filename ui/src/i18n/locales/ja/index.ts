import type { Messages } from "../../types.ts";
import { accounts } from "./accounts.ts";
import { agent } from "./agent.ts";
import { common } from "./common.ts";
import { downloads } from "./downloads.ts";
import { errors } from "./errors.ts";
import { mapWorkbench } from "./mapWorkbench.ts";
import { panels } from "./panels.ts";
import { settings } from "./settings.ts";
import { shared } from "./shared.ts";
import { shell } from "./shell.ts";

export const ja: Messages = { accounts, agent, common, downloads, errors, mapWorkbench, panels, settings, shared, shell };
