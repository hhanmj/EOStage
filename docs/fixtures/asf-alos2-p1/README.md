# ASF ALOS-2 P1 fixtures

These deterministic JSON documents define the proposed ALOS-2 ScanSAR Level
1.1 mapping boundary. They are documentation fixtures, not executable Provider
responses. `example.invalid` locators prevent accidental network use.

`granule-wbd-valid.json` retains a minimal set of non-secret facts from the
pinned ASF `asf_search` fixture at commit
`21f6b5d4bef7cc26dc87e44320f2f18545a3cedc`; all access locators are replaced.
`granule-wbd-cmr-umm.json` retains the corresponding bounded `meta`/`umm`
response shape so loopback tests exercise the production normalization seam;
its access locators and credentials endpoint are non-routable examples. The WBS
case and all negative mutations are synthetic.

No fixture is marked verified production behavior.
