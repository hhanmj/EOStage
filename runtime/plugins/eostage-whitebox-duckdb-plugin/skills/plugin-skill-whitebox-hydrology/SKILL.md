---
name: plugin-skill-whitebox-hydrology
description: 使用本地 WhiteboxTools (runtime/whitebox-cli/whitebox_tools.exe) 执行高精度水文预处理、汇流累积、河网提取与地形湿度指数 (TWI) 分析。
author: EOStage
license: GPL-3.0-or-later
---

# WhiteboxTools 水文地貌与滑坡孕灾因子分析规程

## 1. 标准水文河网提取五步法（`dataset: "whitebox_run"`）
1. **最小代价洼地刻蚀（优先于填洼以保留涵洞/桥梁通道）**：
   - `tool_name: "BreachDepressionsLeastCost"`, `tool_args: ["-i=dem.tif", "-o=dem_breached.tif", "--dist=100", "--fill"]`
2. **D8 流向矩阵计算**：
   - `tool_name: "D8Pointer"`, `tool_args: ["-i=dem_breached.tif", "-o=d8_pntr.tif"]`
3. **D8 汇流累积量（单位集水面积 SCA）**：
   - `tool_name: "D8FlowAccumulation"`, `tool_args: ["-i=dem_breached.tif", "-o=sca.tif", "--out_type=specific contributing area"]`
4. **河网栅格提取**：
   - `tool_name: "ExtractStreams"`, `tool_args: ["--flow_accum=sca.tif", "-o=streams.tif", "--threshold=1000.0"]`
5. **地形湿度指数（WetnessIndex / TWI，滑坡易发性核心因子）**：
   - `tool_name: "WetnessIndex"`, `tool_args: ["--sca=sca.tif", "--slope=slope.tif", "-o=twi.tif"]`
