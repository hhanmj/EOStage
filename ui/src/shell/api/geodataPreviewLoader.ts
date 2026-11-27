import { fetchGeodataPreviewDataUrl } from "../../lib/bridge.ts";
import { createGeodataPreviewQueue, type GeodataPreviewQueue } from "./thumbnailLogic.ts";

export const sharedGeodataPreviewQueue: GeodataPreviewQueue = createGeodataPreviewQueue({
  maxConcurrent: 4,
  fetchDataUrl: async (url: string) => {
    const res = await fetchGeodataPreviewDataUrl(url);
    return res.ok && res.data_url ? res.data_url : null;
  },
});
