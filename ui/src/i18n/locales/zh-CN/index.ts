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

export const zhCN = { accounts, agent, common, downloads, errors, mapWorkbench, panels, settings, shared, shell } as const;
