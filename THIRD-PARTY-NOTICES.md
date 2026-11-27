# Third-Party Notices

This document records third-party source code, bundled skill packages, datasets, and visual assets incorporated into or adapted within the EOStage source tree. Cargo and npm package dependencies are additionally verified by `scripts/audit_tauri_host_licenses.py --mode distribution` against the project's `GPL-3.0-or-later` license policy (`scripts/license_policy.py` and `contracts/license-exceptions.v1.json`).

---

## 1. Incorporated & Adapted Third-Party Code

| Project | License | Upstream Repository / Attribution | Location in EOStage | Notes |
| --- | --- | --- | --- | --- |
| **RustGIS** | Apache-2.0 | https://github.com/opengeos/RustGIS | `crates/insar-gis/src/runtime.rs` | Adapted subprocess scratch-directory budget and timeout policy (`gdal_compat.rs`). Modified by EOStage. |
| **GeoLibre** | MIT | https://github.com/opengeos/GeoLibre (`Copyright (c) 2026 Qiusheng Wu`) | `ui/src/shell/api/channelModels.ts` | Adapted provider preset metadata. Modified by EOStage. |
| **GeoAgent** | MIT | https://github.com/opengeos/GeoAgent (`Copyright (c) 2026 Open Geospatial Solutions`, verified against upstream commit `3cff6db6`) | `ui/src/lib/eoAgentSuite.ts` | Ported/adapted TypeScript utilities for on-demand tool discovery, OAuth PKCE helpers, 4D STAC spatial ranking, confirmation soft-cancel, and static script safety checks. Modified by EOStage. |
| **HDF5 Library (`hdf5-metno-src`)** | BSD-3-Clause-LBNL | https://github.com/HDFGroup/hdf5 (`0.10.2`) | Linked via Cargo crate `hdf5-metno-src` | Full verbatim HDF5 / LBNL / UC LLNL redistribution license and notices included in Section 4.3. |

---

## 2. Bundled Third-Party Geoscience & Utility Skills (`ui/public/skills/`)

The following 12 third-party built-in skill packages retain their original `LICENSE` / `LICENSE.txt` files alongside their `SKILL.md` metadata (the remaining 17 built-in skills under `ui/public/skills/` are authored by EOStage):

| Skill ID | Copyright Holder / Author | License | Upstream Repository / Notes | Directory |
| --- | --- | --- | --- | --- |
| `remote-sensing-analysis` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/remote-sensing-analysis/` |
| `change-detection` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/change-detection/` |
| `terrain-hydrology` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills | `ui/public/skills/terrain-hydrology/` |
| `point-cloud-lidar` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/point-cloud-lidar/` |
| `cartography-geoviz` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills | `ui/public/skills/cartography-geoviz/` |
| `geo-deep-learning` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/geo-deep-learning/` |
| `geostatistics-interpolation` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/geostatistics-interpolation/` |
| `spatial-statistics` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills | `ui/public/skills/spatial-statistics/` |
| `network-accessibility-analysis` | Copyright (c) 2026 Muhammed Enes Duran | MIT | https://github.com/muend/geoai-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/network-accessibility-analysis/` |
| `spatial-sql` | Copyright (c) 2026 Jaak Laineste | MIT | https://github.com/jaakla/openmapstack-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/spatial-sql/` |
| `ecmwf-open-data` | European Centre for Medium-Range Weather Forecasts (ECMWF) | Apache-2.0 | https://github.com/ecmwf/ecmwf-weather-skills (Modified by EOStage for desktop environment compatibility) | `ui/public/skills/ecmwf-open-data/` |
| `skill-creator` | Copyright 2026 Anthropic, PBC. | Apache-2.0 | https://github.com/anthropics/skills | `ui/public/skills/skill-creator/` |

---

## 3. Data Sources & Visual Assets

- **geoBoundaries gbOpen** (`CC BY 4.0`, William & Mary geoLab, https://www.geoboundaries.org/): queried on demand for global administrative AOI selection outside China; a minimal GeoJSON excerpt used exclusively as a unit-test routing fixture resides in `crates/insar-provider/tests/fixtures/geoboundaries/r4-real-units.geojson` with inline CC BY 4.0 attribution.
- **DataV & Tianditu**: queried on demand for China administrative boundaries and map tile overlays; no upstream administrative polygon datasets are redistributed in this repository.
- **Esri Basemap & Wayback Services**: queried on demand for interactive map views and historical Wayback snapshots (`contracts/map-providers.v1.json`); full provider attributions apply:
  - World Imagery & Wayback: *Esri, Maxar, Earthstar Geographics, and the GIS User Community*.
  - World Gray Canvas (Light/Dark): *Esri, HERE, Garmin, © OpenStreetMap contributors*.
  - World Topographic / World Street Map: *Esri, HERE, Garmin, USGS, Intermap, © OpenStreetMap contributors, and the GIS User Community*.
- **Onboarding Globe & Preview Imagery (`ui/public/onboarding-globe/`, `ui/public/onboarding-stage1/`)**:
  - These bundled directories are the only onboarding imagery redistributed in this repository. Any optional external Stage-2 high-resolution atlas pack configured via `VITE_EO_ATLAS_CDN_BASE_URL` is externally hosted and not redistributed here; hosts configuring an external pack are responsible for its imagery rights and attributions (including, where applicable, **© JAXA** for ALOS-2/PALSAR-2, **Polar Geospatial Center / NSF** for ArcticDEM, **© ESA WorldCover project / Contains modified Copernicus Sentinel data** under CC BY 4.0, and **NASA Earth Observatory / VIIRS Black Marble**).
  - Earth textures derived from NASA Visible Earth / Blue Marble Next Generation (public domain) and Natural Earth country outlines (public domain).
  - Optical satellite preview tiles: **Contains modified Copernicus Sentinel data [2024-2026]**.
- **Dependency License Gate**: Verified GPLv3-compatible via `python3 scripts/audit_tauri_host_licenses.py --mode distribution` (0 incompatible dependencies, 0 active waivers).

---

## 4. Full Third-Party License Texts

### 4.1 MIT License (GeoLibre, GeoAgent & Bundled MIT Skills)

```text
MIT License

Copyright (c) 2026 Qiusheng Wu (opengeos/GeoLibre)
Copyright (c) 2026 Open Geospatial Solutions (opengeos/GeoAgent)
Copyright (c) 2026 Muhammed Enes Duran (bundled geoscience skills)
Copyright (c) 2026 Jaak Laineste (spatial-sql skill)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### 4.2 Apache License, Version 2.0 (RustGIS, ECMWF Open Data Skill, Anthropic Skill Creator)

```text
Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright 2026 Anthropic, PBC.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```

### 4.3 HDF5 Software Library License (BSD-3-Clause-LBNL, verbatim from `hdf5-metno-src` 0.10.2 `ext/hdf5/LICENSE`)

```text
Copyright Notice and License Terms for
HDF5 (Hierarchical Data Format 5) Software Library and Utilities
-----------------------------------------------------------------------------

HDF5 (Hierarchical Data Format 5) Software Library and Utilities
Copyright 2006 by The HDF Group.

NCSA HDF5 (Hierarchical Data Format 5) Software Library and Utilities
Copyright 1998-2006 by The Board of Trustees of the University of Illinois.

All rights reserved.

This software library and utilities is covered by the 3-clause BSD License.

Redistribution and use in source and binary forms, with or without
modification, are permitted for any purpose (including commercial purposes)
provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions, and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions, and the following disclaimer in the documentation
   and/or materials provided with the distribution.

3. Neither the name of The HDF Group, the name of the University, nor the
   name of any Contributor may be used to endorse or promote products derived
   from this software without specific prior written permission from
   The HDF Group, the University, or the Contributor, respectively.

DISCLAIMER:
THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
“AS IS” AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR
TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

For further details, please refer to the full license text available
at https://opensource.org/licenses/bsd-3-clause

You are under no obligation whatsoever to provide any bug fixes, patches, or
upgrades to the features, functionality or performance of the source code
("Enhancements") to anyone; however, if you choose to make your Enhancements
available either publicly, or directly to The HDF Group, without imposing a
separate written license agreement for such Enhancements, then you hereby
grant the following license: a non-exclusive, royalty-free perpetual license
to install, use, modify, prepare derivative works, incorporate into other
computer software, distribute, and sublicense such enhancements or derivative
works thereof, in binary and source code form.

-----------------------------------------------------------------------------
-----------------------------------------------------------------------------

Contributors:   National Center for Supercomputing Applications (NCSA) at
the University of Illinois, Fortner Software, Unidata Program Center
(netCDF), The Independent JPEG Group (JPEG), Jean-loup Gailly and Mark Adler
(gzip), and Digital Equipment Corporation (DEC).

-----------------------------------------------------------------------------

Portions of HDF5 were developed with support from the Lawrence Berkeley
National Laboratory (LBNL) and the United States Department of Energy
under Prime Contract No. DE-AC02-05CH11231.

-----------------------------------------------------------------------------

Portions of HDF5 were developed with support from Lawrence Livermore
National Laboratory and the United States Department of Energy under
Prime Contract No. DE-AC52-07NA27344.

-----------------------------------------------------------------------------

Portions of HDF5 were developed with support from the University of
California, Lawrence Livermore National Laboratory (UC LLNL).
The following statement applies to those portions of the product and must
be retained in any redistribution of source code, binaries, documentation,
and/or accompanying materials:

   This work was partially produced at the University of California,
   Lawrence Livermore National Laboratory (UC LLNL) under contract
   no. W-7405-ENG-48 (Contract 48) between the U.S. Department of Energy
   (DOE) and The Regents of the University of California (University)
   for the operation of UC LLNL.

   DISCLAIMER:
   THIS WORK WAS PREPARED AS AN ACCOUNT OF WORK SPONSORED BY AN AGENCY OF
   THE UNITED STATES GOVERNMENT. NEITHER THE UNITED STATES GOVERNMENT NOR
   THE UNIVERSITY OF CALIFORNIA NOR ANY OF THEIR EMPLOYEES, MAKES ANY
   WARRANTY, EXPRESS OR IMPLIED, OR ASSUMES ANY LIABILITY OR RESPONSIBILITY
   FOR THE ACCURACY, COMPLETENESS, OR USEFULNESS OF ANY INFORMATION,
   APPARATUS, PRODUCT, OR PROCESS DISCLOSED, OR REPRESENTS THAT ITS USE
   WOULD NOT INFRINGE PRIVATELY- OWNED RIGHTS. REFERENCE HEREIN TO ANY
   SPECIFIC COMMERCIAL PRODUCTS, PROCESS, OR SERVICE BY TRADE NAME,
   TRADEMARK, MANUFACTURER, OR OTHERWISE, DOES NOT NECESSARILY CONSTITUTE
   OR IMPLY ITS ENDORSEMENT, RECOMMENDATION, OR FAVORING BY THE UNITED
   STATES GOVERNMENT OR THE UNIVERSITY OF CALIFORNIA. THE VIEWS AND
   OPINIONS OF AUTHORS EXPRESSED HEREIN DO NOT NECESSARILY STATE OR REFLECT
   THOSE OF THE UNITED STATES GOVERNMENT OR THE UNIVERSITY OF CALIFORNIA,
   AND SHALL NOT BE USED FOR ADVERTISING OR PRODUCT ENDORSEMENT PURPOSES.

-----------------------------------------------------------------------------

Portions of HDF5 were developed with support from the National Science
Foundation under Federal Award No. 2534078.

-----------------------------------------------------------------------------
```
