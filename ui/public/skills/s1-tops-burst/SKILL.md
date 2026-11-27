---
name: s1-tops-burst
description: "Searches, filters, and extracts single Sentinel-1 IW/EW TOPS bursts by relative orbit (Path), subswath (IW1/IW2/IW3), burst ID, and polarization for rapid interferometric stack assembly."
license: MIT
metadata:
  author: EOStage
---

# Sentinel-1 TOPS Burst-Level Search & Extraction

## Purpose

Standard Sentinel-1 SLC frames span $250\text{ km} \times 170\text{ km}$ (~4.5 GB compressed), while a single TOPS burst covers ~$85\text{ km} \times 20\text{ km}$. Burst-level retrieval ensures consistent burst synchronization across multi-temporal stacks and reduces bandwidth by $> 85\%$.

## Workflow

1. Specify working AOI, date window, flight direction (`ASCENDING` / `DESCENDING`), and relative orbit (`Path`).
2. Group candidate bursts by `(relative_orbit, subswath, burst_index)` to verify multi-epoch temporal completeness before queueing downloads.
