use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub struct ODataPage {
    #[serde(rename = "@odata.count")]
    pub matched_count: Option<u64>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    pub value: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataProduct {
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Collection")]
    pub collection: Option<ODataCollection>,
    #[serde(rename = "ContentLength")]
    pub content_length: Option<u64>,
    #[serde(rename = "ContentDate")]
    pub content_date: Option<ODataContentDate>,
    #[serde(rename = "Online")]
    pub online: Option<bool>,
    #[serde(rename = "GeoFootprint")]
    pub geo_footprint: Option<Value>,
    #[serde(rename = "S3Path")]
    pub s3_path: Option<String>,
    #[serde(rename = "Checksum", default)]
    pub checksums: Vec<ODataChecksum>,
    #[serde(rename = "Attributes", default)]
    pub attributes: Vec<ODataAttribute>,
    #[serde(rename = "Assets", default)]
    pub assets: Vec<ODataAsset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataCollection {
    #[serde(rename = "Name")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataContentDate {
    #[serde(rename = "Start")]
    pub start: Option<String>,
    #[serde(rename = "End")]
    pub end: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataChecksum {
    #[serde(rename = "Algorithm")]
    pub algorithm: Option<String>,
    #[serde(rename = "Value")]
    pub value: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataAttribute {
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Value")]
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ODataAsset {
    #[serde(rename = "Type")]
    pub asset_type: Option<String>,
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "ContentType")]
    pub content_type: Option<String>,
    #[serde(rename = "ContentLength")]
    pub content_length: Option<u64>,
    #[serde(rename = "DownloadLink")]
    pub download_link: Option<String>,
}
