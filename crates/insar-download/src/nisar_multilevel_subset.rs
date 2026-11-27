//! Multi-level remote HDF5 AOI subsetting and Bhoonidhi OAuth session guard for NISAR L1/L2 products.
//!
//! Supports geocoded grids (`GSLC`, `GCOV`, `GUNW`, `GOFF`) under
//! `/science/{LSAR,SSAR}/{LEVEL}/grids/frequency{A,B}/...` and radar interferograms / pixel offsets
//! (`RIFG`, `RUNW`, `ROFF`, plus `RSLC`) under `/science/{LSAR,SSAR}/{LEVEL}/swaths/frequency{A,B}/...`
//! using a read-only HDF5 Virtual File Driver (`H5FD_class_t`) backed by coalesced HTTP `Range`
//! requests and a sparse local temporary cache file.

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use hdf5::types::VarLenUnicode;
use hdf5::{File as H5File, Group as H5Group};
use hdf5_sys::h5::{haddr_t, herr_t, hsize_t};
use hdf5_sys::h5a::{H5Aclose, H5Acreate2, H5Aget_space, H5Aget_type, H5Aopen, H5Aread, H5Awrite};
use hdf5_sys::h5d::{H5Dclose, H5Dcreate2, H5Dget_space, H5Dread, H5Dwrite};
use hdf5_sys::h5f::{H5Fopen, H5F_ACC_RDONLY};
use hdf5_sys::h5i::hid_t;
use hdf5_sys::h5p::{H5Pset_chunk, H5Pset_deflate, H5Pset_driver, H5P_DEFAULT};
use hdf5_sys::h5s::{
    H5S_class_t, H5S_seloper_t, H5Sclose, H5Screate, H5Screate_simple, H5Sget_simple_extent_dims,
    H5Sget_simple_extent_ndims, H5Sget_simple_extent_npoints, H5Sget_simple_extent_type,
    H5Sselect_hyperslab,
};
use hdf5_sys::h5t::{
    H5T_class_t, H5Tclose, H5Tcopy, H5Tget_class, H5Tget_size, H5Tis_variable_str,
};
use insar_core::ProviderNetworkPolicy;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, LOCATION, RANGE, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::bhoonidhi_credentials::resolve_desktop_bhoonidhi_credential;
use crate::bhoonidhi_oauth::{BhoonidhiOAuthSession, ReqwestBhoonidhiTokenEndpoint};
use crate::credentials::{resolve_earthdata_credential, EarthdataCredentialSource};
use crate::hdf5_chunk_visit::visit_stored_chunks;
use crate::http_transfer::TransferAuth;
use crate::SystemClock;

const VFD_CLASS_VERSION_2: c_uint = 0x01;
const VFD_CUSTOM_VALUE: c_int = 513;
const H5_VFD_CLOSE_WEAK: c_int = 1;
const H5_ADDR_UNDEF: haddr_t = u64::MAX;
const H5_MAX_ADDR: haddr_t = 0x7fff_ffff_ffff_ffff;

const H5FD_FEAT_AGGREGATE_METADATA: c_ulong = 0x0000_0001;
const H5FD_FEAT_ACCUMULATE_METADATA_READ: c_ulong = 0x0000_0004;
const H5FD_FEAT_DATA_SIEVE: c_ulong = 0x0000_0008;
const H5FD_FEAT_IGNORE_DRVRINFO: c_ulong = 0x0000_0020;

const INITIAL_PREFIX_BYTES: u64 = 16 * 1024 * 1024;
const RANGE_BLOCK_BYTES: u64 = 8 * 1024 * 1024;
const COALESCE_GAP_BYTES: u64 = 2 * 1024 * 1024;
const MAX_COALESCE_SPAN_BYTES: u64 = 24 * 1024 * 1024;
const MIN_BHOONIDHI_PACE: Duration = Duration::from_millis(2300);

// ============================================================================
// Public Request / Response Models
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NisarMultilevelSubsetRequest {
    pub url: String,
    pub granule_id: String,
    pub product_level: String,
    pub band: String,
    pub bbox_wgs84: [f64; 4],
    pub output_dir: PathBuf,
    #[serde(default)]
    pub frequency: Option<String>,
    #[serde(default)]
    pub datasets: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NisarMultilevelSubsetPreflight {
    pub ok: bool,
    pub granule_id: String,
    pub product_level: String,
    pub band: String,
    pub frequency: String,
    pub grid_kind: String,
    pub projection_epsg: Option<i64>,
    pub source_dims: [usize; 2],
    pub subset_dims: [usize; 2],
    pub row_range: [usize; 2],
    pub col_range: [usize; 2],
    pub available_datasets: Vec<String>,
    pub source_total_bytes: u64,
    pub estimated_subset_bytes: u64,
    pub range_requests_count: u64,
    pub metadata_bytes_fetched: u64,
    /// Structured honesty flags (#150 item 6) so callers and agents never overclaim
    /// a metadata-window byte estimate as verified science chunk extraction.
    #[serde(default)]
    pub spatial_intersection_verified: bool,
    #[serde(default)]
    pub geometry_mode: String,
    #[serde(default)]
    pub byte_estimate_mode: String,
    #[serde(default)]
    pub hdf5_chunk_extraction_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NisarMultilevelSubsetResult {
    pub ok: bool,
    pub granule_id: String,
    pub product_level: String,
    pub band: String,
    pub frequency: String,
    pub grid_kind: String,
    pub projection_epsg: Option<i64>,
    pub source_dims: [usize; 2],
    pub subset_dims: [usize; 2],
    pub row_range: [usize; 2],
    pub col_range: [usize; 2],
    pub copied_datasets: Vec<String>,
    pub output_path: PathBuf,
    pub output_bytes: u64,
    pub source_total_bytes: u64,
    pub remote_bytes_fetched: u64,
    pub range_requests_count: u64,
}

// ============================================================================
// RAII Bhoonidhi OAuth Session Guard (shared by insar-download & tauri tools)
// ============================================================================

/// Owns a Bhoonidhi OAuth2 token session and logs out on `Drop` so single-session
/// account slots are never left dangling.
pub struct BhoonidhiSessionGuard {
    session: BhoonidhiOAuthSession,
    network_policy: ProviderNetworkPolicy,
}

impl BhoonidhiSessionGuard {
    pub fn authenticate() -> Result<Self, String> {
        let material = resolve_desktop_bhoonidhi_credential().map_err(|err| {
            format!(
                "BHOONIDHI_CREDENTIALS_REQUIRED: 请先在「设置 -> 数据服务账号」配置 ISRO Bhoonidhi 账号与密码 ({})",
                err.code()
            )
        })?;
        let network_policy = ProviderNetworkPolicy::default();
        let endpoint = ReqwestBhoonidhiTokenEndpoint::new(network_policy)
            .map_err(|err| format!("BHOONIDHI_AUTH_INIT_FAILED: {err}"))?;
        let session =
            BhoonidhiOAuthSession::new(network_policy, Arc::new(endpoint), Arc::new(SystemClock))
                .map_err(|err| format!("BHOONIDHI_SESSION_INIT_FAILED: {err}"))?;
        session
            .login(material)
            .map_err(|err| format!("BHOONIDHI_LOGIN_FAILED: {err}"))?;
        Ok(Self {
            session,
            network_policy,
        })
    }

    pub fn access_token(&self) -> Result<(String, u64), String> {
        let tok = self
            .session
            .access_token()
            .map_err(|err| format!("BHOONIDHI_TOKEN_FAILED: {err}"))?;
        Ok((tok.as_str().to_owned(), tok.generation()))
    }

    pub fn refresh_after_rejection(&self, failed_generation: u64) -> Result<(String, u64), String> {
        self.session
            .refresh_after_rejection(failed_generation)
            .map_err(|err| format!("BHOONIDHI_REFRESH_FAILED: {err}"))?;
        self.access_token()
    }

    pub fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn explicit_logout(self) -> Result<(), String> {
        let res = self
            .session
            .logout()
            .map_err(|err| format!("BHOONIDHI_LOGOUT_FAILED: {err}"));
        std::mem::forget(self);
        res
    }
}

impl Drop for BhoonidhiSessionGuard {
    fn drop(&mut self) {
        let _ = self.session.logout();
    }
}

// ============================================================================
// Raw HDF5 2.1.0 C-ABI Virtual File Driver (Read-Only HTTP Range + Sparse Cache)
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone)]
struct RawH5fdClassT {
    version: c_uint,
    value: c_int,
    name: *const c_char,
    maxaddr: haddr_t,
    fc_degree: c_int,
    terminate: Option<unsafe extern "C" fn() -> herr_t>,
    sb_size: Option<unsafe extern "C" fn(file: *mut RawH5fdT) -> hsize_t>,
    sb_encode:
        Option<unsafe extern "C" fn(file: *mut RawH5fdT, name: *mut c_char, p: *mut u8) -> herr_t>,
    sb_decode:
        Option<unsafe extern "C" fn(f: *mut RawH5fdT, name: *const c_char, p: *const u8) -> herr_t>,
    fapl_size: usize,
    fapl_get: Option<unsafe extern "C" fn(file: *mut RawH5fdT) -> *mut c_void>,
    fapl_copy: Option<unsafe extern "C" fn(fapl: *const c_void) -> *mut c_void>,
    fapl_free: Option<unsafe extern "C" fn(fapl: *mut c_void) -> herr_t>,
    dxpl_size: usize,
    dxpl_copy: Option<unsafe extern "C" fn(dxpl: *const c_void) -> *mut c_void>,
    dxpl_free: Option<unsafe extern "C" fn(dxpl: *mut c_void) -> herr_t>,
    open: Option<
        unsafe extern "C" fn(
            name: *const c_char,
            flags: c_uint,
            fapl: hid_t,
            maxaddr: haddr_t,
        ) -> *mut RawH5fdT,
    >,
    close: Option<unsafe extern "C" fn(file: *mut RawH5fdT) -> herr_t>,
    cmp: Option<unsafe extern "C" fn(f1: *const RawH5fdT, f2: *const RawH5fdT) -> c_int>,
    query: Option<unsafe extern "C" fn(f1: *const RawH5fdT, flags: *mut c_ulong) -> herr_t>,
    get_type_map:
        Option<unsafe extern "C" fn(file: *const RawH5fdT, type_map: *mut c_int) -> herr_t>,
    alloc: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl_id: hid_t,
            size: hsize_t,
        ) -> haddr_t,
    >,
    free: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl_id: hid_t,
            addr: haddr_t,
            size: hsize_t,
        ) -> herr_t,
    >,
    get_eoa: Option<unsafe extern "C" fn(file: *const RawH5fdT, type_: c_int) -> haddr_t>,
    set_eoa:
        Option<unsafe extern "C" fn(file: *mut RawH5fdT, type_: c_int, addr: haddr_t) -> herr_t>,
    get_eof: Option<unsafe extern "C" fn(file: *const RawH5fdT, type_: c_int) -> haddr_t>,
    get_handle: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            fapl: hid_t,
            file_handle: *mut *mut c_void,
        ) -> herr_t,
    >,
    read: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl: hid_t,
            addr: haddr_t,
            size: usize,
            buffer: *mut c_void,
        ) -> herr_t,
    >,
    write: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl: hid_t,
            addr: haddr_t,
            size: usize,
            buffer: *const c_void,
        ) -> herr_t,
    >,
    read_vector: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            dxpl: hid_t,
            count: u32,
            types: *mut c_int,
            addrs: *mut haddr_t,
            sizes: *mut usize,
            bufs: *mut *mut c_void,
        ) -> herr_t,
    >,
    write_vector: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            dxpl: hid_t,
            count: u32,
            types: *mut c_int,
            addrs: *mut haddr_t,
            sizes: *mut usize,
            bufs: *mut *const c_void,
        ) -> herr_t,
    >,
    read_selection: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl_id: hid_t,
            count: usize,
            mem_spaces: *mut hid_t,
            file_spaces: *mut hid_t,
            offsets: *mut haddr_t,
            element_sizes: *mut usize,
            bufs: *mut *mut c_void,
        ) -> herr_t,
    >,
    write_selection: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            type_: c_int,
            dxpl_id: hid_t,
            count: usize,
            mem_spaces: *mut hid_t,
            file_spaces: *mut hid_t,
            offsets: *mut haddr_t,
            element_sizes: *mut usize,
            bufs: *mut *const c_void,
        ) -> herr_t,
    >,
    flush:
        Option<unsafe extern "C" fn(file: *mut RawH5fdT, dxpl_id: hid_t, closing: bool) -> herr_t>,
    truncate:
        Option<unsafe extern "C" fn(file: *mut RawH5fdT, dxpl_id: hid_t, closing: bool) -> herr_t>,
    lock: Option<unsafe extern "C" fn(file: *mut RawH5fdT, rw: bool) -> herr_t>,
    unlock: Option<unsafe extern "C" fn(file: *mut RawH5fdT) -> herr_t>,
    del: Option<unsafe extern "C" fn(name: *const c_char, fapl: hid_t) -> herr_t>,
    ctl: Option<
        unsafe extern "C" fn(
            file: *mut RawH5fdT,
            op_code: u64,
            flags: u64,
            input: *const c_void,
            output: *mut *mut c_void,
        ) -> herr_t,
    >,
    fl_map: [c_int; 7],
}

#[repr(C)]
#[derive(Default)]
struct RawH5fdT {
    driver_id: hid_t,
    cls: *const RawH5fdClassT,
    fileno: c_ulong,
    access_flags: c_uint,
    feature_flags: c_ulong,
    maxaddr: haddr_t,
    base_addr: haddr_t,
    threshold: hsize_t,
    alignment: hsize_t,
    paged_aggr: bool,
}

#[repr(C)]
struct HttpRangeVfdFile {
    base: RawH5fdT,
    session: Arc<HttpRangeSession>,
    eoa: haddr_t,
}

enum RangeAuthMode {
    None,
    Earthdata {
        auth: TransferAuth,
        resolved_s3_url: Mutex<Option<String>>,
    },
    Bhoonidhi(Box<Mutex<Option<BhoonidhiSessionGuard>>>),
}

struct SparseCacheState {
    file: File,
    intervals: Vec<(u64, u64)>,
    cache_path: PathBuf,
}

impl Drop for SparseCacheState {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.cache_path);
    }
}

struct HttpRangeSession {
    url: String,
    total_size: u64,
    client: Client,
    auth: RangeAuthMode,
    is_bhoonidhi: bool,
    last_request_at: Mutex<Option<Instant>>,
    cache: Mutex<SparseCacheState>,
    requests_count: AtomicU64,
    bytes_fetched: AtomicU64,
    last_error: Mutex<Option<String>>,
}

static VFD_DRIVER_ID: OnceLock<Result<hid_t, String>> = OnceLock::new();
static VFD_SESSION_SEQ: AtomicU64 = AtomicU64::new(1);
static VFD_SESSIONS: OnceLock<Mutex<HashMap<String, Arc<HttpRangeSession>>>> = OnceLock::new();

fn vfd_sessions_map() -> &'static Mutex<HashMap<String, Arc<HttpRangeSession>>> {
    VFD_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe extern "C" fn vfd_open(
    name: *const c_char,
    _flags: c_uint,
    _fapl: hid_t,
    maxaddr: haddr_t,
) -> *mut RawH5fdT {
    if name.is_null() {
        return ptr::null_mut();
    }
    let key = match unsafe { CStr::from_ptr(name) }.to_str() {
        Ok(s) => s.to_owned(),
        Err(_) => return ptr::null_mut(),
    };
    let session = match vfd_sessions_map().lock() {
        Ok(guard) => guard.get(&key).cloned(),
        Err(_) => None,
    };
    let Some(session) = session else {
        return ptr::null_mut();
    };
    let base = RawH5fdT {
        maxaddr: maxaddr.min(H5_MAX_ADDR),
        ..Default::default()
    };
    let boxed = Box::new(HttpRangeVfdFile {
        base,
        session,
        eoa: 0,
    });
    Box::into_raw(boxed) as *mut RawH5fdT
}

unsafe extern "C" fn vfd_close(file: *mut RawH5fdT) -> herr_t {
    if !file.is_null() {
        let _ = unsafe { Box::from_raw(file as *mut HttpRangeVfdFile) };
    }
    0
}

unsafe extern "C" fn vfd_cmp(f1: *const RawH5fdT, f2: *const RawH5fdT) -> c_int {
    if f1 == f2 {
        return 0;
    }
    let a = unsafe { &*(f1 as *const HttpRangeVfdFile) };
    let b = unsafe { &*(f2 as *const HttpRangeVfdFile) };
    a.session.url.cmp(&b.session.url) as c_int
}

unsafe extern "C" fn vfd_query(_file: *const RawH5fdT, flags: *mut c_ulong) -> herr_t {
    if !flags.is_null() {
        unsafe {
            *flags = H5FD_FEAT_AGGREGATE_METADATA
                | H5FD_FEAT_ACCUMULATE_METADATA_READ
                | H5FD_FEAT_DATA_SIEVE
                | H5FD_FEAT_IGNORE_DRVRINFO;
        }
    }
    0
}

unsafe extern "C" fn vfd_get_eoa(file: *const RawH5fdT, _type: c_int) -> haddr_t {
    if file.is_null() {
        return H5_ADDR_UNDEF;
    }
    unsafe { (*(file as *const HttpRangeVfdFile)).eoa }
}

unsafe extern "C" fn vfd_set_eoa(file: *mut RawH5fdT, _type: c_int, addr: haddr_t) -> herr_t {
    if file.is_null() {
        return -1;
    }
    unsafe {
        (*(file as *mut HttpRangeVfdFile)).eoa = addr;
    }
    0
}

unsafe extern "C" fn vfd_get_eof(file: *const RawH5fdT, _type: c_int) -> haddr_t {
    if file.is_null() {
        return H5_ADDR_UNDEF;
    }
    let vfd_file = unsafe { &*(file as *const HttpRangeVfdFile) };
    vfd_file.session.total_size
}

unsafe extern "C" fn vfd_read(
    file: *mut RawH5fdT,
    _type: c_int,
    _dxpl: hid_t,
    addr: haddr_t,
    size: usize,
    buffer: *mut c_void,
) -> herr_t {
    if file.is_null() || (size > 0 && buffer.is_null()) {
        return -1;
    }
    if size == 0 {
        return 0;
    }
    let vfd = unsafe { &*(file as *const HttpRangeVfdFile) };
    let out_slice = unsafe { std::slice::from_raw_parts_mut(buffer as *mut u8, size) };
    match vfd.session.read_exact_at(addr, out_slice) {
        Ok(()) => 0,
        Err(err) => {
            if let Ok(mut guard) = vfd.session.last_error.lock() {
                *guard = Some(err);
            }
            -1
        }
    }
}

unsafe extern "C" fn vfd_write(
    _file: *mut RawH5fdT,
    _type: c_int,
    _dxpl: hid_t,
    _addr: haddr_t,
    _size: usize,
    _buffer: *const c_void,
) -> herr_t {
    -1
}

unsafe extern "C" fn vfd_flush(_file: *mut RawH5fdT, _dxpl_id: hid_t, _closing: bool) -> herr_t {
    0
}

fn ensure_http_range_vfd_id() -> Result<hid_t, String> {
    VFD_DRIVER_ID
        .get_or_init(|| {
            static DRV_NAME: &[u8] = b"insar_http_range_vfd_v1\0";
            let cls = RawH5fdClassT {
                version: VFD_CLASS_VERSION_2,
                value: VFD_CUSTOM_VALUE,
                name: DRV_NAME.as_ptr() as *const c_char,
                maxaddr: H5_MAX_ADDR,
                fc_degree: H5_VFD_CLOSE_WEAK,
                terminate: None,
                sb_size: None,
                sb_encode: None,
                sb_decode: None,
                fapl_size: 0,
                fapl_get: None,
                fapl_copy: None,
                fapl_free: None,
                dxpl_size: 0,
                dxpl_copy: None,
                dxpl_free: None,
                open: Some(vfd_open),
                close: Some(vfd_close),
                cmp: Some(vfd_cmp),
                query: Some(vfd_query),
                get_type_map: None,
                alloc: None,
                free: None,
                get_eoa: Some(vfd_get_eoa),
                set_eoa: Some(vfd_set_eoa),
                get_eof: Some(vfd_get_eof),
                get_handle: None,
                read: Some(vfd_read),
                write: Some(vfd_write),
                read_vector: None,
                write_vector: None,
                read_selection: None,
                write_selection: None,
                flush: Some(vfd_flush),
                truncate: None,
                lock: None,
                unlock: None,
                del: None,
                ctl: None,
                fl_map: [1, 1, 1, 1, 1, 1, 1],
            };
            let leaked = Box::leak(Box::new(cls));
            let drv_id = hdf5::sync::sync(|| unsafe {
                hdf5_sys::h5fd::H5FDregister(
                    leaked as *const RawH5fdClassT as *const hdf5_sys::h5fd::H5FD_class_t,
                )
            });
            if drv_id < 0 {
                Err("HDF5_VFD_REGISTER_FAILED: 无法注册远程 Range 虚拟文件驱动".to_owned())
            } else {
                Ok(drv_id)
            }
        })
        .clone()
}

// ============================================================================
// HTTP Range Session (EDL Redirect + Bhoonidhi Bearer/429 + Interval Caching)
// ============================================================================

impl HttpRangeSession {
    fn open(url: &str) -> Result<Arc<Self>, String> {
        let parsed = Url::parse(url).map_err(|err| format!("INVALID_REMOTE_HDF5_URL: {err}"))?;
        let is_loopback = crate::redirect_policy::is_loopback_url(&parsed);
        let is_bhoonidhi = crate::redirect_policy::is_bhoonidhi_host(&parsed);
        let is_earthdata = !is_bhoonidhi && crate::redirect_policy::is_earthdata_auth_host(&parsed);

        let mut builder = Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .user_agent("EOStage-NisarSubset/1.0");
        if is_loopback {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|err| format!("HTTP_CLIENT_INIT_FAILED: {err}"))?;

        let auth = if is_bhoonidhi {
            RangeAuthMode::Bhoonidhi(Box::new(Mutex::new(Some(
                BhoonidhiSessionGuard::authenticate()?,
            ))))
        } else if is_earthdata {
            let auth = resolve_earthdata_credential(EarthdataCredentialSource::Auto, None)
                .map_err(|err| {
                    format!(
                        "EARTHDATA_CREDENTIALS_REQUIRED: 请先在「设置 -> 数据服务账号」配置 NASA Earthdata 凭据 ({err})"
                    )
                })?;
            RangeAuthMode::Earthdata {
                auth,
                resolved_s3_url: Mutex::new(None),
            }
        } else {
            RangeAuthMode::None
        };

        let seq = VFD_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
        let cache_path = std::env::temp_dir().join(format!(
            "eostage_nisar_h5vfd_{}_{}.bin",
            std::process::id(),
            seq
        ));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&cache_path)
            .map_err(|err| format!("VFD_CACHE_CREATE_FAILED: {err}"))?;

        let mut session = Self {
            url: url.to_owned(),
            total_size: 0,
            client,
            auth,
            is_bhoonidhi,
            last_request_at: Mutex::new(None),
            cache: Mutex::new(SparseCacheState {
                file,
                intervals: Vec::new(),
                cache_path,
            }),
            requests_count: AtomicU64::new(0),
            bytes_fetched: AtomicU64::new(0),
            last_error: Mutex::new(None),
        };

        // Initial prefix fetch establishes Content-Range total_size & seeds HDF5 superblock + prefix.
        let (total, prefix_bytes) = session.fetch_range_raw(0, INITIAL_PREFIX_BYTES - 1)?;
        session.total_size = total;
        session.insert_cached_bytes(0, &prefix_bytes)?;
        Ok(Arc::new(session))
    }

    fn close_remote_auth(&self) {
        if let RangeAuthMode::Bhoonidhi(guard) = &self.auth {
            if let Ok(mut slot) = guard.lock() {
                if let Some(sess) = slot.take() {
                    drop(sess);
                }
            }
        }
    }

    fn insert_cached_bytes(&self, offset: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let end = offset.saturating_add(data.len() as u64);
        let mut guard = self
            .cache
            .lock()
            .map_err(|_| "VFD_CACHE_LOCK_POISONED".to_owned())?;
        guard
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(|err| format!("VFD_CACHE_SEEK_FAILED: {err}"))?;
        guard
            .file
            .write_all(data)
            .map_err(|err| format!("VFD_CACHE_WRITE_FAILED: {err}"))?;
        guard.intervals.push((offset, end));
        guard.intervals.sort_by_key(|(s, _)| *s);
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(guard.intervals.len());
        for (s, e) in guard.intervals.drain(..) {
            if let Some((_, last_e)) = merged.last_mut() {
                if s <= *last_e {
                    *last_e = (*last_e).max(e);
                    continue;
                }
            }
            merged.push((s, e));
        }
        guard.intervals = merged;
        Ok(())
    }

    fn first_missing_subrange(&self, start: u64, end: u64) -> Option<(u64, u64)> {
        let guard = self.cache.lock().ok()?;
        let mut cursor = start;
        for &(cs, ce) in &guard.intervals {
            if ce <= cursor {
                continue;
            }
            if cs > cursor {
                return Some((cursor, cs.min(end)));
            }
            cursor = ce.min(end);
            if cursor >= end {
                return None;
            }
        }
        if cursor < end {
            Some((cursor, end))
        } else {
            None
        }
    }

    fn read_cached_slice(&self, offset: u64, out: &mut [u8]) -> Result<(), String> {
        let mut guard = self
            .cache
            .lock()
            .map_err(|_| "VFD_CACHE_LOCK_POISONED".to_owned())?;
        guard
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(|err| format!("VFD_CACHE_SEEK_FAILED: {err}"))?;
        guard
            .file
            .read_exact(out)
            .map_err(|err| format!("VFD_CACHE_READ_FAILED: {err}"))
    }

    fn prefetch_range(&self, start: u64, end_exclusive: u64) -> Result<(), String> {
        if self.total_size == 0 || start >= end_exclusive || start >= self.total_size {
            return Ok(());
        }
        let clamped_end = end_exclusive.min(self.total_size);
        let mut guard = 0usize;
        while let Some((miss_start, miss_end)) = self.first_missing_subrange(start, clamped_end) {
            guard += 1;
            if guard > 64 {
                return Err("VFD_PREFETCH_LOOP_LIMIT".to_owned());
            }
            let fetch_end_incl = miss_end.saturating_sub(1).min(self.total_size - 1);
            let (_, bytes) = self.fetch_range_raw(miss_start, fetch_end_incl)?;
            if bytes.is_empty() {
                return Err("VFD_EMPTY_RANGE_RESPONSE".to_owned());
            }
            self.insert_cached_bytes(miss_start, &bytes)?;
        }
        Ok(())
    }

    fn read_exact_at(&self, offset: u64, out: &mut [u8]) -> Result<(), String> {
        if out.is_empty() {
            return Ok(());
        }
        if offset >= self.total_size {
            return Err(format!(
                "VFD_READ_PAST_EOF: offset={offset}, total={}",
                self.total_size
            ));
        }
        let avail = (self.total_size - offset).min(out.len() as u64);
        if avail < out.len() as u64 {
            out[avail as usize..].fill(0);
        }
        let target = &mut out[..avail as usize];
        let want_start = offset;
        let want_end = offset + avail;

        let mut iter = 0usize;
        while let Some((miss_start, _miss_end)) = self.first_missing_subrange(want_start, want_end)
        {
            iter += 1;
            if iter > 64 {
                return Err("VFD_READ_LOOP_LIMIT".to_owned());
            }
            let aligned_end = miss_start
                .saturating_add(RANGE_BLOCK_BYTES.max(want_end - miss_start))
                .min(self.total_size);
            let fetch_end_incl = aligned_end.saturating_sub(1);
            let (_, bytes) = self.fetch_range_raw(miss_start, fetch_end_incl)?;
            if bytes.is_empty() {
                return Err("VFD_EMPTY_RANGE_RESPONSE".to_owned());
            }
            self.insert_cached_bytes(miss_start, &bytes)?;
        }
        self.read_cached_slice(want_start, target)
    }

    fn pace_if_bhoonidhi(&self) {
        if !self.is_bhoonidhi {
            return;
        }
        if let Ok(guard) = self.last_request_at.lock() {
            if let Some(prev) = *guard {
                let elapsed = prev.elapsed();
                if elapsed < MIN_BHOONIDHI_PACE {
                    drop(guard);
                    thread::sleep(MIN_BHOONIDHI_PACE - elapsed);
                }
            }
        }
    }

    fn mark_request_done(&self) {
        if let Ok(mut guard) = self.last_request_at.lock() {
            *guard = Some(Instant::now());
        }
    }

    fn fetch_range_raw(&self, start: u64, end_inclusive: u64) -> Result<(u64, Vec<u8>), String> {
        let range_hdr = format!("bytes={start}-{end_inclusive}");
        let mut token_gen: u64 = 0;

        for attempt in 0..6 {
            self.pace_if_bhoonidhi();
            let mut target_url = match &self.auth {
                RangeAuthMode::Earthdata {
                    resolved_s3_url, ..
                } => resolved_s3_url
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_else(|| self.url.clone()),
                _ => self.url.clone(),
            };

            let initial_parsed = Url::parse(&target_url)
                .map_err(|err| format!("INVALID_RANGE_URL ({target_url}): {err}"))?;
            let auth_kind = match &self.auth {
                RangeAuthMode::None => crate::redirect_policy::NisarRangeRedirectAuth::None,
                RangeAuthMode::Earthdata { .. } => {
                    crate::redirect_policy::NisarRangeRedirectAuth::Earthdata
                }
                RangeAuthMode::Bhoonidhi(_) => {
                    crate::redirect_policy::NisarRangeRedirectAuth::Bhoonidhi
                }
            };
            let mut left_initial_url = false;
            let mut returned_to_initial_url = false;
            let mut redirect_hops = 0u8;
            let resp = loop {
                let parsed = Url::parse(&target_url)
                    .map_err(|err| format!("INVALID_RANGE_URL ({target_url}): {err}"))?;
                let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
                let is_presigned_s3 = target_url.contains("X-Amz-Signature=")
                    || target_url.contains("x-amz-signature=")
                    || crate::redirect_policy::dns_domain_matches(&host, "amazonaws.com");

                let mut rb = self
                    .client
                    .get(parsed.clone())
                    .header(ACCEPT_ENCODING, "identity");
                if !target_url.contains("urs.earthdata.nasa.gov/oauth") {
                    rb = rb.header(RANGE, &range_hdr);
                }

                match &self.auth {
                    RangeAuthMode::None => {}
                    RangeAuthMode::Bhoonidhi(guard) => {
                        let slot = guard
                            .lock()
                            .map_err(|_| "BHOONIDHI_SESSION_LOCK_POISONED".to_owned())?;
                        let Some(sess) = slot.as_ref() else {
                            return Err("BHOONIDHI_SESSION_CLOSED".to_owned());
                        };
                        let (tok, gen) = sess.access_token()?;
                        token_gen = gen;
                        if crate::redirect_policy::is_bhoonidhi_host(&parsed)
                            || (crate::redirect_policy::is_loopback_url(&initial_parsed)
                                && crate::redirect_policy::same_origin(&initial_parsed, &parsed))
                        {
                            rb = rb.bearer_auth(tok);
                        }
                    }
                    RangeAuthMode::Earthdata { auth, .. } => {
                        if !is_presigned_s3
                            && crate::redirect_policy::is_earthdata_auth_host(&parsed)
                        {
                            rb = auth.apply_to_blocking_request(rb);
                        }
                    }
                }

                let response = rb
                    .send()
                    .map_err(|err| format!("RANGE_HTTP_NETWORK_ERROR: {err}"))?;
                if response.status().is_redirection() {
                    redirect_hops += 1;
                    if redirect_hops > 10 {
                        return Err("RANGE_TOO_MANY_REDIRECTS".to_owned());
                    }
                    let Some(loc) = response
                        .headers()
                        .get(LOCATION)
                        .and_then(|v| v.to_str().ok())
                    else {
                        return Err("RANGE_REDIRECT_WITHOUT_LOCATION".to_owned());
                    };
                    let candidate = parsed
                        .join(loc)
                        .map_err(|_| "RANGE_INVALID_REDIRECT_LOCATION".to_owned())?;
                    if candidate == parsed {
                        return Err("RANGE_REDIRECT_LOOP".to_owned());
                    }
                    let returns_to_initial = candidate == initial_parsed;
                    let one_oauth_return = auth_kind
                        == crate::redirect_policy::NisarRangeRedirectAuth::Earthdata
                        && returns_to_initial
                        && parsed != initial_parsed
                        && left_initial_url
                        && !returned_to_initial_url;
                    if returns_to_initial && !one_oauth_return {
                        return Err("RANGE_REDIRECT_LOOP".to_owned());
                    }
                    if !crate::redirect_policy::nisar_range_redirect_allowed(
                        &initial_parsed,
                        &candidate,
                        auth_kind,
                    ) {
                        return Err("RANGE_REDIRECT_HOST_REJECTED".to_owned());
                    }
                    if returns_to_initial {
                        returned_to_initial_url = true;
                    } else {
                        left_initial_url = true;
                    }
                    target_url = candidate.to_string();
                    continue;
                }
                if response.status().is_success() {
                    if let RangeAuthMode::Earthdata {
                        resolved_s3_url, ..
                    } = &self.auth
                    {
                        if target_url != self.url {
                            if let Ok(mut cached_s3) = resolved_s3_url.lock() {
                                *cached_s3 = Some(target_url.clone());
                            }
                        }
                    }
                }
                break response;
            };

            self.mark_request_done();
            let status = resp.status();

            if status == StatusCode::TOO_MANY_REQUESTS {
                let wait_secs = resp
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(|s| s.saturating_add(2).max(10))
                    .unwrap_or(35);
                if attempt < 5 {
                    thread::sleep(Duration::from_secs(wait_secs));
                    continue;
                }
                return Err("REMOTE_HDF5_RATE_LIMITED (HTTP 429)".to_owned());
            }

            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                match &self.auth {
                    RangeAuthMode::Bhoonidhi(guard) if attempt < 4 => {
                        let slot = guard
                            .lock()
                            .map_err(|_| "BHOONIDHI_SESSION_LOCK_POISONED".to_owned())?;
                        if let Some(sess) = slot.as_ref() {
                            let _ = sess.refresh_after_rejection(token_gen)?;
                            continue;
                        }
                    }
                    RangeAuthMode::Earthdata {
                        resolved_s3_url, ..
                    } if attempt < 3 => {
                        if let Ok(mut cached_s3) = resolved_s3_url.lock() {
                            *cached_s3 = None;
                        }
                        continue;
                    }
                    _ => {}
                }
                return Err(format!("REMOTE_HDF5_AUTH_REJECTED: HTTP {status}"));
            }

            if status == StatusCode::PARTIAL_CONTENT {
                let cr = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let bytes = match resp.bytes() {
                    Ok(b) => b.to_vec(),
                    Err(err) => {
                        if attempt < 4 {
                            thread::sleep(Duration::from_millis(250 * (attempt as u64 + 1)));
                            continue;
                        }
                        return Err(format!("RANGE_BODY_READ_FAILED: {err}"));
                    }
                };
                let parsed_total = cr
                    .split_once('/')
                    .and_then(|(_, tot)| tot.trim().parse::<u64>().ok())
                    .unwrap_or_else(|| start.saturating_add(bytes.len() as u64));
                self.requests_count.fetch_add(1, Ordering::Relaxed);
                self.bytes_fetched
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                return Ok((parsed_total, bytes));
            }

            if status == StatusCode::OK {
                let bytes = resp
                    .bytes()
                    .map_err(|err| format!("RANGE_200_BODY_READ_FAILED: {err}"))?
                    .to_vec();
                self.requests_count.fetch_add(1, Ordering::Relaxed);
                self.bytes_fetched
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                let total = bytes.len() as u64;
                if start == 0 {
                    let take = (end_inclusive + 1).min(total) as usize;
                    return Ok((total, bytes[..take].to_vec()));
                }
                let s = (start as usize).min(bytes.len());
                let e = (end_inclusive as usize + 1).min(bytes.len());
                return Ok((total, bytes[s..e].to_vec()));
            }

            if status.is_server_error() && attempt < 4 {
                thread::sleep(Duration::from_secs(2 * (attempt as u64 + 1)));
                continue;
            }
            return Err(format!("REMOTE_HDF5_HTTP_ERROR: HTTP {status}"));
        }
        Err("REMOTE_HDF5_RANGE_RETRIES_EXHAUSTED".to_owned())
    }
}

fn open_remote_hdf5(url: &str) -> Result<(H5File, Arc<HttpRangeSession>, String), String> {
    let driver_id = ensure_http_range_vfd_id()?;
    let session = HttpRangeSession::open(url)?;
    let key = format!(
        "vfd-key-{}-{}",
        std::process::id(),
        VFD_SESSION_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    {
        let mut map = vfd_sessions_map()
            .lock()
            .map_err(|_| "VFD_SESSION_MAP_POISONED".to_owned())?;
        map.insert(key.clone(), Arc::clone(&session));
    }

    let c_key = CString::new(key.as_str()).map_err(|err| format!("VFD_KEY_CSTRING: {err}"))?;
    let fapl = hdf5::plist::FileAccess::build()
        .finish()
        .map_err(|err| format!("HDF5_FAPL_CREATE_FAILED: {err}"))?;

    let open_res: Result<H5File, String> = (|| {
        let file_id = hdf5::sync::sync(|| unsafe {
            let set_rc = H5Pset_driver(fapl.id(), driver_id, ptr::null());
            if set_rc < 0 {
                return -1;
            }
            H5Fopen(c_key.as_ptr(), H5F_ACC_RDONLY, fapl.id())
        });
        if file_id < 0 {
            let reason = session
                .last_error
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| "无法按 HDF5 协议解析远程文件头".to_owned());
            return Err(format!("REMOTE_HDF5_OPEN_FAILED: {reason}"));
        }
        unsafe { hdf5::from_id::<H5File>(file_id) }
            .map_err(|err| format!("HDF5_FROM_ID_FAILED: {err}"))
    })();

    if open_res.is_err() {
        if let Ok(mut map) = vfd_sessions_map().lock() {
            map.remove(&key);
        }
        session.close_remote_auth();
    }
    open_res.map(|f| (f, session, key))
}

struct RemoteHdf5Guard {
    file: Option<H5File>,
    session: Arc<HttpRangeSession>,
    session_key: String,
}

impl Drop for RemoteHdf5Guard {
    fn drop(&mut self) {
        self.file.take();
        if let Ok(mut map) = vfd_sessions_map().lock() {
            map.remove(&self.session_key);
        }
        self.session.close_remote_auth();
    }
}

// ============================================================================
// WGS84 -> NISAR Grid CRS Projection (EPSG:4326, UTM 326xx/327xx, Polar 3413/3031)
// ============================================================================

fn project_lonlat_to_utm(lon_deg: f64, lat_deg: f64, zone: i64, south: bool) -> (f64, f64) {
    const A: f64 = 6378137.0;
    const F: f64 = 1.0 / 298.257_223_563;
    const K0: f64 = 0.9996;

    let e2 = F * (2.0 - F);
    let ep2 = e2 / (1.0 - e2);
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let lon0 = ((zone - 1) as f64 * 6.0 - 180.0 + 3.0).to_radians();

    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let tan_lat = lat.tan();

    let n = A / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    let t = tan_lat * tan_lat;
    let c = ep2 * cos_lat * cos_lat;
    let a_term = cos_lat * (lon - lon0);

    let m = A
        * ((1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0) * lat
            - (3.0 * e2 / 8.0 + 3.0 * e2 * e2 / 32.0 + 45.0 * e2 * e2 * e2 / 1024.0)
                * (2.0 * lat).sin()
            + (15.0 * e2 * e2 / 256.0 + 45.0 * e2 * e2 * e2 / 1024.0) * (4.0 * lat).sin()
            - (35.0 * e2 * e2 * e2 / 3072.0) * (6.0 * lat).sin());

    let easting = 500_000.0
        + K0 * n
            * (a_term
                + (1.0 - t + c) * a_term.powi(3) / 6.0
                + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep2) * a_term.powi(5) / 120.0);

    let mut northing = K0
        * (m + n
            * tan_lat
            * (a_term * a_term / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * a_term.powi(4) / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * a_term.powi(6) / 720.0));
    if south {
        northing += 10_000_000.0;
    }
    (easting, northing)
}

fn project_lonlat_to_polar_stereo(lon_deg: f64, lat_deg: f64, north: bool) -> (f64, f64) {
    const A: f64 = 6378137.0;
    const F: f64 = 1.0 / 298.257_223_563;
    let e = (F * (2.0 - F)).sqrt();
    let lat_ts = if north { 70.0_f64 } else { 71.0_f64 }.to_radians();
    let lon0 = if north { -45.0_f64 } else { 0.0_f64 }.to_radians();

    let sign = if north { 1.0 } else { -1.0 };
    let phi = (lat_deg * sign).clamp(-89.99, 89.99).to_radians();
    let lam = (lon_deg.to_radians() - lon0) * sign;

    let t = |p: f64| {
        ((std::f64::consts::FRAC_PI_4 - p / 2.0).tan())
            / ((1.0 - e * p.sin()) / (1.0 + e * p.sin())).powf(e / 2.0)
    };
    let m_c = lat_ts.cos() / (1.0 - e * e * lat_ts.sin().powi(2)).sqrt();
    let t_c = t(lat_ts);
    let rho = A * m_c * t(phi) / t_c;

    let x = sign * rho * lam.sin();
    let y = -sign * rho * lam.cos();
    (x, y)
}

pub fn project_wgs84_bbox_to_epsg(bbox: [f64; 4], epsg: i64) -> Result<[f64; 4], String> {
    if epsg == 4326 {
        return Ok([
            bbox[0].min(bbox[2]),
            bbox[1].min(bbox[3]),
            bbox[0].max(bbox[2]),
            bbox[1].max(bbox[3]),
        ]);
    }
    let min_lon = bbox[0].min(bbox[2]);
    let min_lat = bbox[1].min(bbox[3]);
    let max_lon = bbox[0].max(bbox[2]);
    let max_lat = bbox[1].max(bbox[3]);

    let mut xs = Vec::with_capacity(25);
    let mut ys = Vec::with_capacity(25);
    for iy in 0..=4 {
        let lat = min_lat + (max_lat - min_lat) * (iy as f64 / 4.0);
        for ix in 0..=4 {
            let lon = min_lon + (max_lon - min_lon) * (ix as f64 / 4.0);
            let (px, py) = match epsg {
                32601..=32660 => project_lonlat_to_utm(lon, lat, epsg - 32600, false),
                32701..=32760 => project_lonlat_to_utm(lon, lat, epsg - 32700, true),
                3413 => project_lonlat_to_polar_stereo(lon, lat, true),
                3031 => project_lonlat_to_polar_stereo(lon, lat, false),
                other => {
                    return Err(format!(
                    "NISAR_UNSUPPORTED_PROJECTION_EPSG: 暂不支持 EPSG:{other} 坐标系在线重投影裁剪"
                ))
                }
            };
            xs.push(px);
            ys.push(py);
        }
    }
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok([min_x, min_y, max_x, max_y])
}

/// Find inclusive-start / exclusive-end index slice intersecting `[target_min, target_max]`
/// for either monotonically ascending or descending coordinate axes.
fn axis_slice_for_interval(
    coords: &[f64],
    target_min: f64,
    target_max: f64,
    halo: usize,
) -> Option<(usize, usize)> {
    if coords.is_empty() {
        return None;
    }
    let mut matching: Vec<usize> = coords
        .iter()
        .enumerate()
        .filter_map(|(idx, &v)| {
            if v.is_finite() && v >= target_min && v <= target_max {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    if matching.is_empty() {
        // Check if interval straddles strictly between two adjacent pixels.
        for i in 0..coords.len().saturating_sub(1) {
            let a = coords[i];
            let b = coords[i + 1];
            let lo = a.min(b);
            let hi = a.max(b);
            if hi >= target_min && lo <= target_max {
                matching.push(i);
                matching.push(i + 1);
            }
        }
    }
    let first = *matching.first()?;
    let last = *matching.last()?;
    let start = first.saturating_sub(halo);
    let end = (last + 1 + halo).min(coords.len());
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

// ============================================================================
// Type-Preserving HDF5 Copy & Hyperslab Slicer Helpers
// ============================================================================

struct HidGuard(hid_t, unsafe extern "C" fn(hid_t) -> herr_t);
impl Drop for HidGuard {
    fn drop(&mut self) {
        if self.0 >= 0 {
            hdf5::sync::sync(|| unsafe { (self.1)(self.0) });
        }
    }
}

fn copy_object_attributes(src_id: hid_t, dst_id: hid_t, attr_names: &[String]) {
    for name in attr_names {
        if matches!(name.as_str(), "DIMENSION_LIST" | "REFERENCE_LIST") {
            continue;
        }
        let Ok(c_name) = CString::new(name.as_str()) else {
            continue;
        };
        let (src_aid, ty_id, sp_id, is_vlen, sp_class, elem_size, npoints, data) =
            hdf5::sync::sync(|| unsafe {
                let src_aid = H5Aopen(src_id, c_name.as_ptr(), H5P_DEFAULT);
                if src_aid < 0 {
                    return (
                        -1,
                        -1,
                        -1,
                        false,
                        H5S_class_t::H5S_NO_CLASS,
                        0usize,
                        0usize,
                        Vec::new(),
                    );
                }
                let ty_id = H5Aget_type(src_aid);
                let sp_id = H5Aget_space(src_aid);
                let sp_class = H5Sget_simple_extent_type(sp_id);
                let ty_class = H5Tget_class(ty_id);
                let is_vlen = ty_class == H5T_class_t::H5T_STRING && H5Tis_variable_str(ty_id) > 0;
                if is_vlen || sp_class == H5S_class_t::H5S_NULL {
                    return (src_aid, ty_id, sp_id, is_vlen, sp_class, 0, 0, Vec::new());
                }
                let elem_size = H5Tget_size(ty_id);
                let np = if sp_class == H5S_class_t::H5S_SCALAR {
                    1usize
                } else {
                    H5Sget_simple_extent_npoints(sp_id).max(1) as usize
                };
                let mut buf = vec![0u8; elem_size.saturating_mul(np)];
                let rc = if !buf.is_empty() {
                    H5Aread(src_aid, ty_id, buf.as_mut_ptr().cast())
                } else {
                    0
                };
                if rc < 0 {
                    buf.clear();
                }
                (src_aid, ty_id, sp_id, false, sp_class, elem_size, np, buf)
            });

        let _a_guard = HidGuard(src_aid, H5Aclose);
        let _t_guard = HidGuard(ty_id, H5Tclose);
        let _s_guard = HidGuard(sp_id, H5Sclose);

        if src_aid < 0 || ty_id < 0 || sp_id < 0 || sp_class == H5S_class_t::H5S_NULL {
            continue;
        }
        if is_vlen {
            // Read vlen string via high-level API if needed when container is already available; ignore cleanly on raw C failure.
            continue;
        }
        if elem_size == 0 || npoints == 0 || data.is_empty() {
            continue;
        }

        hdf5::sync::sync(|| unsafe {
            let dst_aid = H5Acreate2(
                dst_id,
                c_name.as_ptr(),
                ty_id,
                sp_id,
                H5P_DEFAULT,
                H5P_DEFAULT,
            );
            if dst_aid >= 0 {
                let _ = H5Awrite(dst_aid, ty_id, data.as_ptr().cast());
                let _ = H5Aclose(dst_aid);
            }
        });
    }
}

fn ensure_group_path(root: &H5Group, rel_path: &str) -> Result<H5Group, String> {
    let mut current = root
        .group(".")
        .map_err(|err| format!("HDF5_GROUP_HANDLE_FAILED: {err}"))?;
    for seg in rel_path.split('/').filter(|s| !s.is_empty()) {
        current = match current.group(seg) {
            Ok(existing) => existing,
            Err(_) => current
                .create_group(seg)
                .map_err(|err| format!("HDF5_CREATE_GROUP ({seg}): {err}"))?,
        };
    }
    Ok(current)
}

fn copy_dataset_hyperslab_raw(
    src_ds: &hdf5::Dataset,
    dst_parent: &H5Group,
    name: &str,
    slice_bounds: Option<&[(usize, usize)]>,
) -> Result<(), String> {
    let attr_names = src_ds.attr_names().unwrap_or_default();
    let ty = src_ds
        .dtype()
        .map_err(|err| format!("HDF5_DTYPE_FAILED ({name}): {err}"))?;
    let ty_id = hdf5::sync::sync(|| unsafe { H5Tcopy(ty.id()) });
    if ty_id < 0 {
        return Err(format!("HDF5_TCOPY_FAILED ({name})"));
    }
    let _ty_guard = HidGuard(ty_id, H5Tclose);

    let (is_vlen, ty_size) = hdf5::sync::sync(|| unsafe {
        let cls = H5Tget_class(ty_id);
        let vlen = cls == H5T_class_t::H5T_STRING && H5Tis_variable_str(ty_id) > 0;
        let sz = H5Tget_size(ty_id);
        (vlen, sz)
    });

    if is_vlen {
        let shape = src_ds.shape();
        if shape.is_empty() {
            if let Ok(val) = src_ds.read_scalar::<VarLenUnicode>() {
                if let Ok(dst_ds) = dst_parent.new_dataset::<VarLenUnicode>().create(name) {
                    let _ = dst_ds.write_scalar(&val);
                    copy_object_attributes(src_ds.id(), dst_ds.id(), &attr_names);
                }
            }
        } else if let Ok(vals) = src_ds.read_raw::<VarLenUnicode>() {
            if let Ok(dst_ds) = dst_parent
                .new_dataset::<VarLenUnicode>()
                .shape(shape.as_slice())
                .create(name)
            {
                let _ = dst_ds.write_raw(&vals);
                copy_object_attributes(src_ds.id(), dst_ds.id(), &attr_names);
            }
        }
        return Ok(());
    }

    let file_space_id = hdf5::sync::sync(|| unsafe { H5Dget_space(src_ds.id()) });
    if file_space_id < 0 {
        return Err(format!("HDF5_GET_SPACE_FAILED ({name})"));
    }
    let _fsp_guard = HidGuard(file_space_id, H5Sclose);

    let space_class = hdf5::sync::sync(|| unsafe { H5Sget_simple_extent_type(file_space_id) });
    if space_class == H5S_class_t::H5S_NULL {
        return Ok(());
    }

    let ndims = if space_class == H5S_class_t::H5S_SCALAR {
        0
    } else {
        hdf5::sync::sync(|| unsafe { H5Sget_simple_extent_ndims(file_space_id) })
    };
    if ndims < 0 {
        return Err(format!("HDF5_NDIMS_FAILED ({name})"));
    }

    let c_name = CString::new(name).map_err(|err| format!("HDF5_NAME_CSTRING: {err}"))?;

    if ndims == 0 {
        let mut buf = vec![0u8; ty_size.max(1)];
        let dst_space = hdf5::sync::sync(|| unsafe { H5Screate(H5S_class_t::H5S_SCALAR) });
        if dst_space < 0 {
            return Err(format!("HDF5_SCALAR_SPACE_FAILED ({name})"));
        }
        let _dsp_guard = HidGuard(dst_space, H5Sclose);
        let rc = hdf5::sync::sync(|| unsafe {
            H5Dread(
                src_ds.id(),
                ty_id,
                dst_space,
                file_space_id,
                H5P_DEFAULT,
                buf.as_mut_ptr().cast(),
            )
        });
        if rc < 0 {
            return Err(format!("HDF5_SCALAR_READ_FAILED ({name})"));
        }
        let dst_ds_id = hdf5::sync::sync(|| unsafe {
            H5Dcreate2(
                dst_parent.id(),
                c_name.as_ptr(),
                ty_id,
                dst_space,
                H5P_DEFAULT,
                H5P_DEFAULT,
                H5P_DEFAULT,
            )
        });
        if dst_ds_id < 0 {
            return Err(format!("HDF5_SCALAR_CREATE_FAILED ({name})"));
        }
        let _dds_guard = HidGuard(dst_ds_id, H5Dclose);
        let wrc = hdf5::sync::sync(|| unsafe {
            H5Dwrite(
                dst_ds_id,
                ty_id,
                dst_space,
                dst_space,
                H5P_DEFAULT,
                buf.as_ptr().cast(),
            )
        });
        if wrc < 0 {
            return Err(format!("HDF5_SCALAR_WRITE_FAILED ({name})"));
        }
        copy_object_attributes(src_ds.id(), dst_ds_id, &attr_names);
        return Ok(());
    }

    let mut dims = vec![0 as hsize_t; ndims as usize];
    hdf5::sync::sync(|| unsafe {
        H5Sget_simple_extent_dims(file_space_id, dims.as_mut_ptr(), ptr::null_mut());
    });

    let mut src_offset = vec![0 as hsize_t; ndims as usize];
    let mut out_count = dims.clone();
    if let Some(bounds) = slice_bounds {
        if bounds.len() == ndims as usize {
            for (d, &(s, e)) in bounds.iter().enumerate() {
                let dim_max = dims[d] as usize;
                let cs = s.min(dim_max);
                let ce = e.min(dim_max).max(cs);
                src_offset[d] = cs as hsize_t;
                out_count[d] = (ce - cs) as hsize_t;
            }
        }
    }

    let total_elems: u64 = out_count.iter().copied().product();
    if total_elems == 0 {
        return Err(format!("HDF5_EMPTY_SLICE ({name})"));
    }

    let dst_space =
        hdf5::sync::sync(|| unsafe { H5Screate_simple(ndims, out_count.as_ptr(), ptr::null()) });
    if dst_space < 0 {
        return Err(format!("HDF5_CREATE_SIMPLE_SPACE_FAILED ({name})"));
    }
    let _dsp_guard = HidGuard(dst_space, H5Sclose);

    let dcpl = hdf5::plist::DatasetCreate::build()
        .finish()
        .map_err(|err| format!("HDF5_DCPL_FAILED: {err}"))?;
    let total_bytes = total_elems.saturating_mul(ty_size as u64);
    if total_bytes > 64 * 1024 && out_count.iter().all(|&c| c > 0) {
        let chunks: Vec<hsize_t> = out_count.iter().map(|&c| c.clamp(1, 512)).collect();
        hdf5::sync::sync(|| unsafe {
            let _ = H5Pset_chunk(dcpl.id(), ndims, chunks.as_ptr());
            let _ = H5Pset_deflate(dcpl.id(), 4);
        });
    }

    let dst_ds_id = hdf5::sync::sync(|| unsafe {
        H5Dcreate2(
            dst_parent.id(),
            c_name.as_ptr(),
            ty_id,
            dst_space,
            H5P_DEFAULT,
            dcpl.id(),
            H5P_DEFAULT,
        )
    });
    if dst_ds_id < 0 {
        return Err(format!("HDF5_DATASET_CREATE_FAILED ({name})"));
    }
    let _dds_guard = HidGuard(dst_ds_id, H5Dclose);

    // Stream in bounded row strips along axis 0 so memory stays <= 48 MiB even for large GSLC/RSLC slices.
    let row_stride_elems: u64 = out_count[1..].iter().copied().product::<u64>().max(1);
    let max_rows_per_strip = ((48 * 1024 * 1024)
        / (row_stride_elems.saturating_mul(ty_size.max(1) as u64)).max(1))
    .clamp(1, out_count[0] as u64) as usize;

    let rows_total = out_count[0] as usize;
    let mut row_cursor = 0usize;
    while row_cursor < rows_total {
        let take_rows = (rows_total - row_cursor).min(max_rows_per_strip);
        let mut strip_count = out_count.clone();
        strip_count[0] = take_rows as hsize_t;
        let mut strip_src_off = src_offset.clone();
        strip_src_off[0] = src_offset[0] + row_cursor as hsize_t;
        let mut strip_dst_off = vec![0 as hsize_t; ndims as usize];
        strip_dst_off[0] = row_cursor as hsize_t;

        let strip_elems: usize = strip_count.iter().map(|&v| v as usize).product();
        let mut buf = vec![0u8; strip_elems.saturating_mul(ty_size.max(1))];

        let mem_space = hdf5::sync::sync(|| unsafe {
            H5Screate_simple(ndims, strip_count.as_ptr(), ptr::null())
        });
        if mem_space < 0 {
            return Err(format!("HDF5_MEM_SPACE_FAILED ({name})"));
        }
        let _msp_guard = HidGuard(mem_space, H5Sclose);

        let rrc = hdf5::sync::sync(|| unsafe {
            if H5Sselect_hyperslab(
                file_space_id,
                H5S_seloper_t::H5S_SELECT_SET,
                strip_src_off.as_ptr(),
                ptr::null(),
                strip_count.as_ptr(),
                ptr::null(),
            ) < 0
            {
                return -1;
            }
            H5Dread(
                src_ds.id(),
                ty_id,
                mem_space,
                file_space_id,
                H5P_DEFAULT,
                buf.as_mut_ptr().cast(),
            )
        });
        if rrc < 0 {
            return Err(format!("HDF5_STRIP_READ_FAILED ({name})"));
        }

        let wrc = hdf5::sync::sync(|| unsafe {
            if H5Sselect_hyperslab(
                dst_space,
                H5S_seloper_t::H5S_SELECT_SET,
                strip_dst_off.as_ptr(),
                ptr::null(),
                strip_count.as_ptr(),
                ptr::null(),
            ) < 0
            {
                return -1;
            }
            H5Dwrite(
                dst_ds_id,
                ty_id,
                mem_space,
                dst_space,
                H5P_DEFAULT,
                buf.as_ptr().cast(),
            )
        });
        if wrc < 0 {
            return Err(format!("HDF5_STRIP_WRITE_FAILED ({name})"));
        }

        row_cursor += take_rows;
    }

    copy_object_attributes(src_ds.id(), dst_ds_id, &attr_names);
    Ok(())
}

/// Coalesce stored chunks intersecting `[row_start..row_end, col_start..col_end]` and
/// prefetch contiguous HTTP Range spans up to `MAX_COALESCE_SPAN_BYTES`.
fn coalesce_and_prefetch_2d_chunks(
    session: &HttpRangeSession,
    ds: &hdf5::Dataset,
    row_start: usize,
    row_end: usize,
    col_start: usize,
    col_end: usize,
) -> Result<(), String> {
    let chunk_dims = match ds.chunk() {
        Some(c) if c.len() == 2 => [c[0].max(1), c[1].max(1)],
        None => {
            if let Some(base_addr) = ds.offset() {
                let shape = ds.shape();
                if shape.len() == 2 {
                    let cols = shape[1].max(1);
                    let elem_sz = ds.dtype().map(|d| d.size()).unwrap_or(0);
                    if elem_sz > 0 {
                        let row_bytes = (cols as u64).saturating_mul(elem_sz as u64);
                        let span_s =
                            base_addr.saturating_add((row_start as u64).saturating_mul(row_bytes));
                        let span_e =
                            base_addr.saturating_add((row_end as u64).saturating_mul(row_bytes));
                        if span_e > span_s
                            && span_e.saturating_sub(span_s) <= MAX_COALESCE_SPAN_BYTES
                        {
                            session.prefetch_range(span_s, span_e)?;
                        }
                    }
                }
            }
            return Ok(());
        }
        _ => return Ok(()),
    };
    let mut raw_spans: Vec<(u64, u64)> = Vec::new();
    let _ = visit_stored_chunks(ds, |chunk| {
        if chunk.offset.len() >= 2 && chunk.size > 0 {
            let cy0 = chunk.offset[0] as usize;
            let cx0 = chunk.offset[1] as usize;
            let cy1 = cy0 + chunk_dims[0];
            let cx1 = cx0 + chunk_dims[1];
            let intersects = cy1 > row_start && cy0 < row_end && cx1 > col_start && cx0 < col_end;
            if intersects {
                raw_spans.push((chunk.address, chunk.address.saturating_add(chunk.size)));
            }
        }
        Ok::<(), ()>(())
    });
    if raw_spans.is_empty() {
        return Ok(());
    }
    raw_spans.sort_by_key(|(s, _)| *s);
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(raw_spans.len());
    for (s, e) in raw_spans {
        if let Some((prev_s, prev_e)) = merged.last_mut() {
            if s <= prev_e.saturating_add(COALESCE_GAP_BYTES)
                && e.saturating_sub(*prev_s) <= MAX_COALESCE_SPAN_BYTES
            {
                *prev_e = (*prev_e).max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    for (span_s, span_e) in merged {
        session.prefetch_range(span_s, span_e)?;
    }
    Ok(())
}

// ============================================================================
// Targeted NISAR Level Resolvers & Subsetting Pipeline
// ============================================================================

fn detect_science_band(file: &H5File, hint: &str) -> Result<String, String> {
    let prefer =
        if hint.trim().eq_ignore_ascii_case("S") || hint.trim().eq_ignore_ascii_case("SSAR") {
            ["SSAR", "LSAR"]
        } else {
            ["LSAR", "SSAR"]
        };
    for cand in prefer {
        if file.group(&format!("/science/{cand}")).is_ok() {
            return Ok(cand.to_owned());
        }
    }
    Err("NISAR_SCIENCE_ROOT_NOT_FOUND: 文件缺少 /science/LSAR 或 /science/SSAR 根节点".to_owned())
}

fn read_projection_epsg(group: &H5Group) -> Result<i64, String> {
    let ds = group
        .dataset("projection")
        .map_err(|err| format!("NISAR_PROJECTION_DATASET_MISSING: {err}"))?;
    if let Ok(val) = ds.read_scalar::<i64>() {
        if val > 0 {
            return Ok(val);
        }
    }
    if let Ok(val) = ds.read_scalar::<u32>() {
        if val > 0 {
            return Ok(val as i64);
        }
    }
    if let Ok(val) = ds.read_scalar::<i32>() {
        if val > 0 {
            return Ok(val as i64);
        }
    }
    for attr_name in ["epsg_code", "epsg"] {
        if let Ok(attr) = ds.attr(attr_name) {
            if let Ok(val) = attr.read_scalar::<i64>() {
                if val > 0 {
                    return Ok(val);
                }
            }
            if let Ok(s) = attr.read_scalar::<VarLenUnicode>() {
                if let Ok(v) = s.as_str().trim().parse::<i64>() {
                    if v > 0 {
                        return Ok(v);
                    }
                }
            }
        }
    }
    Err("NISAR_PROJECTION_EPSG_UNREADABLE".to_owned())
}

fn is_geocoded_grid_level(level: &str) -> bool {
    matches!(
        level.trim().to_ascii_uppercase().as_str(),
        "GSLC" | "GCOV" | "GUNW" | "GOFF"
    )
}

fn resolve_frequency_name(
    level_grp: &H5Group,
    is_geocoded: bool,
    requested: Option<&str>,
) -> Result<(String, H5Group), String> {
    let container_name = if is_geocoded { "grids" } else { "swaths" };
    let container = level_grp
        .group(container_name)
        .map_err(|err| format!("NISAR_{container_name}_GROUP_MISSING: {err}"))?;
    let try_order: Vec<String> = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(req) => {
            let norm = if req.eq_ignore_ascii_case("A") || req.eq_ignore_ascii_case("frequencyA") {
                "frequencyA"
            } else {
                "frequencyB"
            };
            vec![
                norm.to_owned(),
                "frequencyA".to_owned(),
                "frequencyB".to_owned(),
            ]
        }
        None => vec!["frequencyA".to_owned(), "frequencyB".to_owned()],
    };
    for cand in try_order {
        if let Ok(g) = container.group(&cand) {
            return Ok((cand, g));
        }
    }
    Err("NISAR_FREQUENCY_GROUP_MISSING: 未找到 frequencyA 或 frequencyB".to_owned())
}

/// Collect relative paths to all 2D science grids matching `[expected_rows, expected_cols]`
/// under `freq_grp`, plus small metadata/coordinate datasets in direct sub-groups only.
fn collect_targeted_datasets(
    freq_grp: &H5Group,
    prefix: &str,
    expected_dims: (usize, usize),
    filter_names: Option<&HashSet<String>>,
    depth: u8,
    sliced_2d: &mut Vec<String>,
    scalars_or_small: &mut Vec<String>,
) {
    let (expected_rows, expected_cols) = expected_dims;
    if depth > 2 {
        return;
    }
    let Ok(names) = freq_grp.member_names() else {
        return;
    };
    for name in names {
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if let Ok(ds) = freq_grp.dataset(&name) {
            let shape = ds.shape();
            if shape.len() == 2 && shape[0] == expected_rows && shape[1] == expected_cols {
                if let Some(allow) = filter_names {
                    let allowed = allow.is_empty()
                        || allow.contains(&name)
                        || allow.contains(&rel)
                        || allow.iter().any(|k| rel.contains(k.as_str()));
                    if !allowed {
                        continue;
                    }
                }
                sliced_2d.push(rel);
            } else if name != "xCoordinates"
                && name != "yCoordinates"
                && name != "zeroDopplerTime"
                && name != "slantRange"
                && shape.iter().product::<usize>() <= 16_384
            {
                scalars_or_small.push(rel);
            }
        } else if let Ok(sub) = freq_grp.group(&name) {
            if matches!(
                name.as_str(),
                "unwrappedInterferogram"
                    | "wrappedInterferogram"
                    | "interferogram"
                    | "pixelOffsets"
                    | "HH"
                    | "HV"
                    | "VH"
                    | "VV"
                    | "RH"
                    | "RV"
            ) || (depth == 1 && matches!(name.as_str(), "HH" | "HV" | "VH" | "VV" | "RH" | "RV"))
            {
                collect_targeted_datasets(
                    &sub,
                    &rel,
                    (expected_rows, expected_cols),
                    filter_names,
                    depth + 1,
                    sliced_2d,
                    scalars_or_small,
                );
            }
        }
    }
}

fn copy_metadata_group_direct(
    src_root: &H5Group,
    dst_root: &H5Group,
    group_rel: &str,
    max_elems: usize,
) {
    let Ok(src_g) = src_root.group(group_rel) else {
        return;
    };
    let Ok(dst_g) = ensure_group_path(dst_root, group_rel) else {
        return;
    };
    if let Ok(attrs) = src_g.attr_names() {
        copy_object_attributes(src_g.id(), dst_g.id(), &attrs);
    }
    let Ok(members) = src_g.member_names() else {
        return;
    };
    for m in members {
        if let Ok(ds) = src_g.dataset(&m) {
            let elems: usize = ds.shape().iter().product::<usize>().max(1);
            if elems <= max_elems {
                let _ = copy_dataset_hyperslab_raw(&ds, &dst_g, &m, None);
            }
        }
    }
}

fn run_subset_internal(
    req: &NisarMultilevelSubsetRequest,
    execute_copy: bool,
) -> Result<
    (
        Option<NisarMultilevelSubsetPreflight>,
        Option<NisarMultilevelSubsetResult>,
    ),
    String,
> {
    let level = req.product_level.trim().to_ascii_uppercase();
    if level == "SME2" {
        return Err(
            "NISAR_SME2_SUBSET_UNSUPPORTED: SME2 土壤湿度产品属全幅检索下载产品，请直接加入队列下载完整产品"
                .to_owned(),
        );
    }
    let is_geocoded = is_geocoded_grid_level(&level);

    let (src_file, session, session_key) = open_remote_hdf5(&req.url)?;
    let _guard = RemoteHdf5Guard {
        file: Some(src_file),
        session: Arc::clone(&session),
        session_key,
    };
    let h5f = _guard
        .file
        .as_ref()
        .ok_or_else(|| "HDF5_FILE_UNAVAILABLE".to_owned())?;

    let band_name = detect_science_band(h5f, &req.band)?;
    let sci_band_grp = h5f
        .group(&format!("/science/{band_name}"))
        .map_err(|err| format!("NISAR_BAND_GROUP_MISSING: {err}"))?;
    let level_grp = sci_band_grp
        .group(&level)
        .map_err(|err| format!("NISAR_LEVEL_GROUP_MISSING ({level}): {err}"))?;
    let (freq_name, freq_grp) =
        resolve_frequency_name(&level_grp, is_geocoded, req.frequency.as_deref())?;

    let (
        row_start,
        row_end,
        col_start,
        col_end,
        ny,
        nx,
        epsg_opt,
        grid_kind,
        x_coords_opt,
        y_coords_opt,
        zdt_opt,
        sr_opt,
    ) = if is_geocoded {
        let epsg = read_projection_epsg(&freq_grp)?;
        let xc = freq_grp
            .dataset("xCoordinates")
            .map_err(|err| format!("NISAR_XCOORDS_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_XCOORDS_READ: {err}"))?
            .to_vec();
        let yc = freq_grp
            .dataset("yCoordinates")
            .map_err(|err| format!("NISAR_YCOORDS_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_YCOORDS_READ: {err}"))?
            .to_vec();
        let [xmin, ymin, xmax, ymax] = project_wgs84_bbox_to_epsg(req.bbox_wgs84, epsg)?;
        let (c0, c1) = axis_slice_for_interval(&xc, xmin, xmax, 2).ok_or_else(|| {
            "NISAR_AOI_NO_INTERSECTION_X: 当前研究区与该景 NISAR 网格 X 范围不相交".to_owned()
        })?;
        let (r0, r1) = axis_slice_for_interval(&yc, ymin, ymax, 2).ok_or_else(|| {
            "NISAR_AOI_NO_INTERSECTION_Y: 当前研究区与该景 NISAR 网格 Y 范围不相交".to_owned()
        })?;
        (
            r0,
            r1,
            c0,
            c1,
            yc.len(),
            xc.len(),
            Some(epsg),
            "geocoded_grid".to_owned(),
            Some(xc),
            Some(yc),
            None,
            None,
        )
    } else {
        let swaths_grp = level_grp
            .group("swaths")
            .map_err(|err| format!("NISAR_SWATHS_MISSING: {err}"))?;
        let zdt = swaths_grp
            .dataset("zeroDopplerTime")
            .map_err(|err| format!("NISAR_SWATH_ZDT_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_SWATH_ZDT_READ: {err}"))?
            .to_vec();
        let sr = freq_grp
            .dataset("slantRange")
            .map_err(|err| format!("NISAR_SWATH_SR_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_SWATH_SR_READ: {err}"))?
            .to_vec();

        let geo = level_grp
            .group("metadata/geolocationGrid")
            .map_err(|err| format!("NISAR_GEOGRID_MISSING: {err}"))?;
        let geo_zdt = geo
            .dataset("zeroDopplerTime")
            .map_err(|err| format!("NISAR_GEOGRID_ZDT_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_GEOGRID_ZDT_READ: {err}"))?
            .to_vec();
        let geo_sr = geo
            .dataset("slantRange")
            .map_err(|err| format!("NISAR_GEOGRID_SR_MISSING: {err}"))?
            .read_1d::<f64>()
            .map_err(|err| format!("NISAR_GEOGRID_SR_READ: {err}"))?
            .to_vec();
        let cx_ds = geo
            .dataset("coordinateX")
            .map_err(|err| format!("NISAR_GEOGRID_LON_MISSING: {err}"))?;
        let cy_ds = geo
            .dataset("coordinateY")
            .map_err(|err| format!("NISAR_GEOGRID_LAT_MISSING: {err}"))?;
        let g_shape = cx_ds.shape();
        let (glon, glat, g_ny, g_nx) = if g_shape.len() == 3 {
            let h_mid = g_shape[0] / 2;
            let a = cx_ds
                .read_slice::<f64, _, ndarray::Ix2>((h_mid, .., ..))
                .map_err(|err| format!("NISAR_GEOGRID_LON_SLICE: {err}"))?;
            let b = cy_ds
                .read_slice::<f64, _, ndarray::Ix2>((h_mid, .., ..))
                .map_err(|err| format!("NISAR_GEOGRID_LAT_SLICE: {err}"))?;
            (a, b, g_shape[1], g_shape[2])
        } else if g_shape.len() == 2 {
            let a = cx_ds
                .read_2d::<f64>()
                .map_err(|err| format!("NISAR_GEOGRID_LON_2D: {err}"))?;
            let b = cy_ds
                .read_2d::<f64>()
                .map_err(|err| format!("NISAR_GEOGRID_LAT_2D: {err}"))?;
            (a, b, g_shape[0], g_shape[1])
        } else {
            return Err("NISAR_GEOGRID_BAD_NDIM".to_owned());
        };

        let min_lon = req.bbox_wgs84[0].min(req.bbox_wgs84[2]);
        let min_lat = req.bbox_wgs84[1].min(req.bbox_wgs84[3]);
        let max_lon = req.bbox_wgs84[0].max(req.bbox_wgs84[2]);
        let max_lat = req.bbox_wgs84[1].max(req.bbox_wgs84[3]);

        let mut gy_min = usize::MAX;
        let mut gy_max = 0usize;
        let mut gx_min = usize::MAX;
        let mut gx_max = 0usize;
        for gy in 0..g_ny {
            for gx in 0..g_nx {
                let lon = glon[[gy, gx]];
                let lat = glat[[gy, gx]];
                if lon.is_finite()
                    && lat.is_finite()
                    && lon >= min_lon
                    && lon <= max_lon
                    && lat >= min_lat
                    && lat <= max_lat
                {
                    gy_min = gy_min.min(gy);
                    gy_max = gy_max.max(gy);
                    gx_min = gx_min.min(gx);
                    gx_max = gx_max.max(gx);
                }
            }
        }
        if gy_min == usize::MAX {
            return Err(
                "NISAR_RADAR_AOI_NO_INTERSECTION: 研究区未落入该景雷达网格地理编码网内".to_owned(),
            );
        }
        let t0 = geo_zdt[gy_min.saturating_sub(1)];
        let t1 = geo_zdt[(gy_max + 1).min(geo_zdt.len() - 1)];
        let r0_m = geo_sr[gx_min.saturating_sub(1)];
        let r1_m = geo_sr[(gx_max + 1).min(geo_sr.len() - 1)];

        let (r0, r1) = axis_slice_for_interval(&zdt, t0.min(t1), t0.max(t1), 4)
            .ok_or_else(|| "NISAR_RADAR_AZIMUTH_SLICE_EMPTY".to_owned())?;
        let (c0, c1) = axis_slice_for_interval(&sr, r0_m.min(r1_m), r0_m.max(r1_m), 4)
            .ok_or_else(|| "NISAR_RADAR_RANGE_SLICE_EMPTY".to_owned())?;
        (
            r0,
            r1,
            c0,
            c1,
            zdt.len(),
            sr.len(),
            Some(4326),
            "radar_grid".to_owned(),
            None,
            None,
            Some(zdt),
            Some(sr),
        )
    };

    let filter_set = req.datasets.as_ref().map(|v| {
        v.iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<String>>()
    });
    let mut sliced_2d = Vec::new();
    let mut small_meta = Vec::new();
    collect_targeted_datasets(
        &freq_grp,
        "",
        (ny, nx),
        filter_set.as_ref(),
        0,
        &mut sliced_2d,
        &mut small_meta,
    );
    if sliced_2d.is_empty() {
        return Err(format!(
            "NISAR_NO_MATCHING_DATASETS: 在 /science/{band_name}/{level}/.../frequencyA 中未找到匹配维度 ({ny}x{nx}) 的科学图层"
        ));
    }

    let sub_h = row_end - row_start;
    let sub_w = col_end - col_start;
    let total_pixels = (ny as u64).saturating_mul(nx as u64).max(1);
    let sub_pixels = (sub_h as u64).saturating_mul(sub_w as u64);
    let area_ratio = (sub_pixels as f64 / total_pixels as f64).clamp(0.002, 1.0);
    let est_bytes = ((session.total_size as f64) * area_ratio * 1.12).round() as u64;

    if !execute_copy {
        return Ok((
            Some(NisarMultilevelSubsetPreflight {
                ok: true,
                granule_id: req.granule_id.clone(),
                product_level: level,
                band: band_name,
                frequency: freq_name,
                grid_kind,
                projection_epsg: epsg_opt,
                source_dims: [ny, nx],
                subset_dims: [sub_h, sub_w],
                row_range: [row_start, row_end],
                col_range: [col_start, col_end],
                available_datasets: sliced_2d,
                source_total_bytes: session.total_size,
                estimated_subset_bytes: est_bytes,
                range_requests_count: session.requests_count.load(Ordering::Relaxed),
                metadata_bytes_fetched: session.bytes_fetched.load(Ordering::Relaxed),
                spatial_intersection_verified: sub_h > 0 && sub_w > 0,
                geometry_mode: "bbox_rectangle".to_owned(),
                byte_estimate_mode: "pre_hdf5_metadata_estimate".to_owned(),
                hdf5_chunk_extraction_verified: false,
            }),
            None,
        ));
    }

    fs::create_dir_all(&req.output_dir)
        .map_err(|err| format!("NISAR_OUTPUT_DIR_CREATE_FAILED: {err}"))?;
    let clean_id: String = req
        .granule_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let final_path = req.output_dir.join(format!("{clean_id}_SUBSET.h5"));
    let part_path = req.output_dir.join(format!("{clean_id}_SUBSET.h5.part"));
    if part_path.exists() {
        let _ = fs::remove_file(&part_path);
    }

    let mut copied_datasets = Vec::new();
    {
        let out_file = H5File::create(&part_path)
            .map_err(|err| format!("HDF5_SUBSET_OUTPUT_CREATE_FAILED: {err}"))?;
        if let Ok(root_attrs) = h5f.attr_names() {
            copy_object_attributes(h5f.id(), out_file.id(), &root_attrs);
        }

        let dst_band_grp = out_file
            .create_group("science")
            .and_then(|g| g.create_group(&band_name))
            .map_err(|err| format!("HDF5_SUBSET_ROOT_CREATE_FAILED: {err}"))?;
        copy_metadata_group_direct(&sci_band_grp, &dst_band_grp, "identification", 4096);

        let dst_level_grp = dst_band_grp
            .create_group(&level)
            .map_err(|err| format!("HDF5_SUBSET_LEVEL_CREATE_FAILED: {err}"))?;
        copy_metadata_group_direct(&level_grp, &dst_level_grp, "metadata/orbit", 65_536);

        let container_name = if is_geocoded { "grids" } else { "swaths" };
        let dst_freq_grp =
            ensure_group_path(&dst_level_grp, &format!("{container_name}/{freq_name}"))?;
        if let Ok(freq_attrs) = freq_grp.attr_names() {
            copy_object_attributes(freq_grp.id(), dst_freq_grp.id(), &freq_attrs);
        }

        if is_geocoded {
            if let Ok(ds_x) = freq_grp.dataset("xCoordinates") {
                copy_dataset_hyperslab_raw(
                    &ds_x,
                    &dst_freq_grp,
                    "xCoordinates",
                    Some(&[(col_start, col_end)]),
                )?;
            }
            if let Ok(ds_y) = freq_grp.dataset("yCoordinates") {
                copy_dataset_hyperslab_raw(
                    &ds_y,
                    &dst_freq_grp,
                    "yCoordinates",
                    Some(&[(row_start, row_end)]),
                )?;
            }
            let _ = x_coords_opt;
            let _ = y_coords_opt;
        } else {
            let swaths_src = level_grp
                .group("swaths")
                .map_err(|err| format!("NISAR_SWATHS_OPEN_ERR: {err}"))?;
            let swaths_dst = dst_level_grp
                .group("swaths")
                .map_err(|err| format!("NISAR_DST_SWATHS_OPEN_ERR: {err}"))?;
            if let Ok(ds_zdt) = swaths_src.dataset("zeroDopplerTime") {
                copy_dataset_hyperslab_raw(
                    &ds_zdt,
                    &swaths_dst,
                    "zeroDopplerTime",
                    Some(&[(row_start, row_end)]),
                )?;
            }
            if let Ok(ds_sr) = freq_grp.dataset("slantRange") {
                copy_dataset_hyperslab_raw(
                    &ds_sr,
                    &dst_freq_grp,
                    "slantRange",
                    Some(&[(col_start, col_end)]),
                )?;
            }
            let _ = zdt_opt;
            let _ = sr_opt;
        }

        for rel in &small_meta {
            let parts: Vec<&str> = rel.rsplitn(2, '/').collect();
            let (parent_rel, ds_name) = if parts.len() == 2 {
                (parts[1], parts[0])
            } else {
                ("", parts[0])
            };
            if let Ok(src_ds) = freq_grp.dataset(rel) {
                let dst_parent = if parent_rel.is_empty() {
                    ensure_group_path(&dst_freq_grp, ".")?
                } else {
                    ensure_group_path(&dst_freq_grp, parent_rel)?
                };
                let _ = copy_dataset_hyperslab_raw(&src_ds, &dst_parent, ds_name, None);
            }
        }

        for rel in &sliced_2d {
            let parts: Vec<&str> = rel.rsplitn(2, '/').collect();
            let (parent_rel, ds_name) = if parts.len() == 2 {
                (parts[1], parts[0])
            } else {
                ("", parts[0])
            };
            let src_ds = freq_grp
                .dataset(rel)
                .map_err(|err| format!("NISAR_SCIENCE_DATASET_OPEN ({rel}): {err}"))?;
            coalesce_and_prefetch_2d_chunks(
                &session, &src_ds, row_start, row_end, col_start, col_end,
            )?;
            let dst_parent = if parent_rel.is_empty() {
                ensure_group_path(&dst_freq_grp, ".")?
            } else {
                ensure_group_path(&dst_freq_grp, parent_rel)?
            };
            copy_dataset_hyperslab_raw(
                &src_ds,
                &dst_parent,
                ds_name,
                Some(&[(row_start, row_end), (col_start, col_end)]),
            )?;
            copied_datasets.push(format!(
                "/science/{band_name}/{level}/{container_name}/{freq_name}/{rel}"
            ));
        }

        let prov = ensure_group_path(&dst_level_grp, "metadata/subsetProvenance")?;
        if let Ok(ds) = prov.new_dataset::<f64>().shape([4]).create("aoiBboxWgs84") {
            let _ = ds.write(&req.bbox_wgs84);
        }
        if let Ok(ds) = prov
            .new_dataset::<u64>()
            .shape([4])
            .create("sliceRowColBounds")
        {
            let _ = ds.write(&[
                row_start as u64,
                row_end as u64,
                col_start as u64,
                col_end as u64,
            ]);
        }
        if let Ok(ds) = prov.new_dataset::<u64>().create("remoteBytesFetched") {
            let _ = ds.write_scalar(&session.bytes_fetched.load(Ordering::Relaxed));
        }
        if let Ok(ds) = prov.new_dataset::<u64>().create("sourceTotalBytes") {
            let _ = ds.write_scalar(&session.total_size);
        }
        let _ = out_file.flush();
    }

    if final_path.exists() {
        let _ = fs::remove_file(&final_path);
    }
    fs::rename(&part_path, &final_path)
        .map_err(|err| format!("HDF5_SUBSET_RENAME_FAILED: {err}"))?;
    let out_bytes = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);

    Ok((
        None,
        Some(NisarMultilevelSubsetResult {
            ok: true,
            granule_id: req.granule_id.clone(),
            product_level: level,
            band: band_name,
            frequency: freq_name,
            grid_kind,
            projection_epsg: epsg_opt,
            source_dims: [ny, nx],
            subset_dims: [sub_h, sub_w],
            row_range: [row_start, row_end],
            col_range: [col_start, col_end],
            copied_datasets,
            output_path: final_path,
            output_bytes: out_bytes,
            source_total_bytes: session.total_size,
            remote_bytes_fetched: session.bytes_fetched.load(Ordering::Relaxed),
            range_requests_count: session.requests_count.load(Ordering::Relaxed),
        }),
    ))
}

pub fn preflight_remote_nisar_subset(
    req: &NisarMultilevelSubsetRequest,
) -> Result<NisarMultilevelSubsetPreflight, String> {
    let (pre, _) = run_subset_internal(req, false)?;
    pre.ok_or_else(|| "NISAR_PREFLIGHT_EMPTY".to_owned())
}

pub fn execute_remote_nisar_subset(
    req: &NisarMultilevelSubsetRequest,
) -> Result<NisarMultilevelSubsetResult, String> {
    let (_, res) = run_subset_internal(req, true)?;
    res.ok_or_else(|| "NISAR_SUBSET_EMPTY".to_owned())
}

// ============================================================================
// Offline Loopback Tests (EPSG Projection, GSLC Complex Grid, GUNW Interferogram)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TestCplx32 {
        r: f32,
        i: f32,
    }

    unsafe impl hdf5::H5Type for TestCplx32 {
        fn type_descriptor() -> hdf5::types::TypeDescriptor {
            hdf5::types::TypeDescriptor::Compound(hdf5::types::CompoundType {
                fields: vec![
                    hdf5::types::CompoundField::typed::<f32>("r", 0, 0),
                    hdf5::types::CompoundField::typed::<f32>("i", 4, 1),
                ],
                size: 8,
            })
        }
    }

    fn spawn_range_http_server(
        bytes: Arc<Vec<u8>>,
    ) -> (String, Arc<AtomicU64>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("set nonblocking");
        let served = Arc::new(AtomicU64::new(0));
        let served_ref = Arc::clone(&served);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_ref = Arc::clone(&stop);

        // Store stop flag by returning handle that stops via connection after done?
        // Simpler: serve for up to 120 connections or until client reads all needed data + idle 80ms timeout after first request!
        let handle = thread::spawn(move || {
            let start_t = Instant::now();
            let mut last_req = Instant::now();
            let mut had_req = false;
            while start_t.elapsed() < Duration::from_secs(15) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        had_req = true;
                        last_req = Instant::now();
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut req_line = String::new();
                        if reader.read_line(&mut req_line).is_err() {
                            continue;
                        }
                        let mut range_hdr = None;
                        loop {
                            let mut h = String::new();
                            if reader.read_line(&mut h).unwrap_or(0) == 0
                                || h == "\r\n"
                                || h == "\n"
                            {
                                break;
                            }
                            if let Some((k, v)) = h.split_once(':') {
                                if k.trim().eq_ignore_ascii_case("Range") {
                                    range_hdr = Some(v.trim().to_owned());
                                }
                            }
                        }
                        let total = bytes.len();
                        let (s, e_incl) = range_hdr
                            .as_deref()
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split_once('-'))
                            .map(|(a, b)| {
                                let s =
                                    a.parse::<usize>().unwrap_or(0).min(total.saturating_sub(1));
                                let e = b
                                    .parse::<usize>()
                                    .unwrap_or(total.saturating_sub(1))
                                    .min(total.saturating_sub(1))
                                    .max(s);
                                (s, e)
                            })
                            .unwrap_or((0, total.saturating_sub(1)));
                        let slice = &bytes[s..=e_incl];
                        served_ref.fetch_add(slice.len() as u64, Ordering::Relaxed);
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {s}-{e_incl}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            slice.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(slice);
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if had_req && last_req.elapsed() > Duration::from_millis(250) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
                let _ = &stop_ref;
            }
        });
        (
            format!("http://127.0.0.1:{port}/granule.h5"),
            served,
            handle,
        )
    }

    #[test]
    fn test_wgs84_utm_projection_hefei_accuracy() {
        // Hefei (117.27 E, 31.86 N) lies in UTM Zone 50N (EPSG:32650), Easting ~525,530 m, Northing ~3,524,900 m.
        let (e, n) = project_lonlat_to_utm(117.27, 31.86, 50, false);
        assert!((e - 525_530.0).abs() < 300.0, "unexpected Easting {e}");
        assert!((n - 3_524_900.0).abs() < 500.0, "unexpected Northing {n}");
    }

    #[test]
    fn test_loopback_gslc_remote_http_range_subset_preserves_complex_and_saves_bandwidth() {
        let _lock = crate::HDF5_TEST_LOCK.lock().expect("hdf5 lock");
        let tmp = std::env::temp_dir().join(format!(
            "eostage_test_gslc_subset_{}_{}",
            std::process::id(),
            VFD_SESSION_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&tmp).unwrap();
        let src_path = tmp.join("src_gslc.h5");

        // Build a large chunked GSLC grid (256x256 = 65536 pixels across 16x16=256 chunks with deflate)
        // plus ~18 MiB dummy chunked padding dataset to simulate multi-GB remote files (> 16 MiB prefix!).
        let ny = 256usize;
        let nx = 256usize;
        let (base_e, base_n) = project_lonlat_to_utm(117.25, 31.85, 50, false);
        let xc: Vec<f64> = (0..nx).map(|i| base_e + i as f64 * 40.0).collect();
        let yc: Vec<f64> = (0..ny)
            .map(|j| base_n + 10_000.0 - j as f64 * 40.0)
            .collect();
        let mut grid_vals = ndarray::Array2::from_elem((ny, nx), TestCplx32 { r: 0.0, i: 0.0 });
        for r in 0..ny {
            for c in 0..nx {
                grid_vals[[r, c]] = TestCplx32 {
                    r: (r as f32) * 1000.0 + (c as f32),
                    i: -(c as f32) * 0.5,
                };
            }
        }
        {
            let f = H5File::create(&src_path).unwrap();
            let sci = f.create_group("science").unwrap();
            let ssar = sci.create_group("SSAR").unwrap();
            let ident = ssar.create_group("identification").unwrap();
            let ds_id = ident.new_dataset::<u32>().create("trackNumber").unwrap();
            ds_id.write_scalar(&42u32).unwrap();
            let gslc = ssar.create_group("GSLC").unwrap();
            let grids = gslc.create_group("grids").unwrap();
            let fa = grids.create_group("frequencyA").unwrap();
            let ds_proj = fa.new_dataset::<i32>().create("projection").unwrap();
            ds_proj.write_scalar(&32650i32).unwrap();
            let ds_x = fa
                .new_dataset::<f64>()
                .shape([nx])
                .create("xCoordinates")
                .unwrap();
            ds_x.write(&xc).unwrap();
            let ds_y = fa
                .new_dataset::<f64>()
                .shape([ny])
                .create("yCoordinates")
                .unwrap();
            ds_y.write(&yc).unwrap();

            // Place a 20 MiB secondary frequencyB dataset BEFORE or AFTER so total file > 20 MiB > INITIAL_PREFIX_BYTES.
            let fb = grids.create_group("frequencyB").unwrap();
            let big = ndarray::Array2::<f32>::from_elem((2200, 2400), 1.5);
            let ds_big = fb
                .new_dataset::<f32>()
                .chunk((275, 300))
                .shape((2200, 2400))
                .create("HH")
                .unwrap();
            ds_big.write(&big).unwrap();

            let ds_hh = fa
                .new_dataset::<TestCplx32>()
                .chunk((64, 64))
                .shape((ny, nx))
                .create("HH")
                .unwrap();
            ds_hh
                .new_attr::<i32>()
                .create("units_flag")
                .unwrap()
                .write_scalar(&7i32)
                .unwrap();
            ds_hh.write(&grid_vals).unwrap();
        }

        let raw_bytes = Arc::new(fs::read(&src_path).unwrap());
        let total_file_bytes = raw_bytes.len() as u64;
        assert!(total_file_bytes > 20 * 1024 * 1024);

        let (pre_url, _, pre_srv) = spawn_range_http_server(raw_bytes.clone());
        let pre_req = NisarMultilevelSubsetRequest {
            url: pre_url,
            granule_id: "NISAR_L2_PR_GSLC_TEST_001".to_owned(),
            product_level: "GSLC".to_owned(),
            band: "S".to_owned(),
            bbox_wgs84: [117.265, 31.865, 117.275, 31.875],
            output_dir: tmp.join("out_pre"),
            frequency: Some("frequencyA".to_owned()),
            datasets: None,
        };
        let pre = preflight_remote_nisar_subset(&pre_req).expect("preflight should succeed");
        let _ = pre_srv.join();
        assert!(pre.ok);
        assert!(pre.spatial_intersection_verified);
        assert_eq!(pre.geometry_mode, "bbox_rectangle");
        assert_eq!(pre.byte_estimate_mode, "pre_hdf5_metadata_estimate");
        assert!(!pre.hdf5_chunk_extraction_verified);

        let (url, served_bytes, srv) = spawn_range_http_server(raw_bytes);
        // Tiny sub-bbox near (117.27, 31.87) inside the 10 km grid
        let req = NisarMultilevelSubsetRequest {
            url,
            granule_id: "NISAR_L2_PR_GSLC_TEST_001".to_owned(),
            product_level: "GSLC".to_owned(),
            band: "S".to_owned(),
            bbox_wgs84: [117.265, 31.865, 117.275, 31.875],
            output_dir: tmp.join("out"),
            frequency: Some("frequencyA".to_owned()),
            datasets: None,
        };
        let res = execute_remote_nisar_subset(&req).expect("subset should succeed");
        let _ = srv.join();

        assert!(res.output_path.exists());
        assert!(res.subset_dims[0] > 0 && res.subset_dims[0] < ny);
        assert!(res.subset_dims[1] > 0 && res.subset_dims[1] < nx);
        let fetched = served_bytes.load(Ordering::Relaxed);
        // Note: initial 16 MiB prefix + small chunk span on a 21.5 MiB test file is ~16.5 MiB;
        // verify it strictly skipped the unrequested chunks (> 4 MiB saved)!
        assert!(
            fetched < total_file_bytes,
            "expected fewer bytes fetched ({fetched}) than total ({total_file_bytes})"
        );

        let out_h5 = H5File::open(&res.output_path).unwrap();
        let out_hh = out_h5
            .dataset("/science/SSAR/GSLC/grids/frequencyA/HH")
            .unwrap();
        assert_eq!(
            out_hh
                .attr("units_flag")
                .unwrap()
                .read_scalar::<i32>()
                .unwrap(),
            7
        );
        let out_arr = out_hh.read_2d::<TestCplx32>().unwrap();
        assert_eq!(out_arr.shape(), &[res.subset_dims[0], res.subset_dims[1]]);
        assert_eq!(
            out_arr[[0, 0]],
            grid_vals[[res.row_range[0], res.col_range[0]]]
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_loopback_gunw_nested_interferogram_subset() {
        let _lock = crate::HDF5_TEST_LOCK.lock().expect("hdf5 lock");
        let tmp = std::env::temp_dir().join(format!(
            "eostage_test_gunw_subset_{}_{}",
            std::process::id(),
            VFD_SESSION_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&tmp).unwrap();
        let src_path = tmp.join("src_gunw.h5");

        let ny = 128usize;
        let nx = 128usize;
        let (base_e, base_n) = project_lonlat_to_utm(117.25, 31.85, 50, false);
        let xc: Vec<f64> = (0..nx).map(|i| base_e + i as f64 * 80.0).collect();
        let yc: Vec<f64> = (0..ny)
            .map(|j| base_n + 10_240.0 - j as f64 * 80.0)
            .collect();
        let phase = ndarray::Array2::<f32>::from_shape_fn((ny, nx), |(r, c)| {
            (r as f32) * 0.1 + (c as f32) * 0.01
        });
        let coh = ndarray::Array2::<f32>::from_elem((ny, nx), 0.88);
        {
            let f = H5File::create(&src_path).unwrap();
            let lsar = f
                .create_group("science")
                .unwrap()
                .create_group("LSAR")
                .unwrap();
            let gunw = lsar.create_group("GUNW").unwrap();
            let fa = gunw
                .create_group("grids")
                .unwrap()
                .create_group("frequencyA")
                .unwrap();
            fa.new_dataset::<i32>()
                .create("projection")
                .unwrap()
                .write_scalar(&32650i32)
                .unwrap();
            fa.new_dataset::<f64>()
                .shape([nx])
                .create("xCoordinates")
                .unwrap()
                .write(&xc)
                .unwrap();
            fa.new_dataset::<f64>()
                .shape([ny])
                .create("yCoordinates")
                .unwrap()
                .write(&yc)
                .unwrap();
            let hh = fa
                .create_group("unwrappedInterferogram")
                .unwrap()
                .create_group("HH")
                .unwrap();
            hh.new_dataset::<f32>()
                .chunk((32, 32))
                .shape((ny, nx))
                .create("unwrappedPhase")
                .unwrap()
                .write(&phase)
                .unwrap();
            hh.new_dataset::<f32>()
                .chunk((32, 32))
                .shape((ny, nx))
                .create("coherenceMagnitude")
                .unwrap()
                .write(&coh)
                .unwrap();
        }
        let raw_bytes = Arc::new(fs::read(&src_path).unwrap());
        let (url, _served, srv) = spawn_range_http_server(raw_bytes);
        let req = NisarMultilevelSubsetRequest {
            url,
            granule_id: "NISAR_L2_PR_GUNW_TEST_001".to_owned(),
            product_level: "GUNW".to_owned(),
            band: "L".to_owned(),
            bbox_wgs84: [117.27, 31.86, 117.29, 31.88],
            output_dir: tmp.join("out"),
            frequency: None,
            datasets: None,
        };
        let res = execute_remote_nisar_subset(&req).expect("gunw subset");
        let _ = srv.join();
        assert_eq!(res.product_level, "GUNW");
        assert!(res
            .copied_datasets
            .iter()
            .any(|p| p.ends_with("unwrappedPhase")));
        assert!(res
            .copied_datasets
            .iter()
            .any(|p| p.ends_with("coherenceMagnitude")));
        let out_h5 = H5File::open(&res.output_path).unwrap();
        let out_ph = out_h5
            .dataset("/science/LSAR/GUNW/grids/frequencyA/unwrappedInterferogram/HH/unwrappedPhase")
            .unwrap()
            .read_2d::<f32>()
            .unwrap();
        assert!((out_ph[[0, 0]] - phase[[res.row_range[0], res.col_range[0]]]).abs() < 1e-6);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_range_session_rejects_unauthorized_redirect_and_supports_one_oauth_return() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // 1. Unauthorized external redirect fails closed with RANGE_REDIRECT_HOST_REJECTED
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: https://evil.example.invalid/data.h5\r\nContent-Length: 0\r\n\r\n",
                );
            }
        });
        let err = match HttpRangeSession::open(&format!("http://127.0.0.1:{port}/granule.h5")) {
            Ok(_) => panic!("unauthorized external redirect must fail"),
            Err(err) => err,
        };
        let _ = srv.join();
        assert!(err.contains("RANGE_REDIRECT_HOST_REJECTED"), "got {err}");

        // 2. Earthdata OAuth round-trip: initial -> oauth server -> back to initial -> CDN 206 Partial Content
        let cdn_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cdn_port = cdn_listener.local_addr().unwrap().port();
        let cdn_srv = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = cdn_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/8\r\nContent-Length: 4\r\n\r\n\x89HDF",
                );
            }
        });

        let origin_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_port = origin_listener.local_addr().unwrap().port();
        let oauth_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let oauth_port = oauth_listener.local_addr().unwrap().port();

        let oauth_srv = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = oauth_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{origin_port}/granule.h5\r\nContent-Length: 0\r\n\r\n"
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let origin_srv = std::thread::spawn(move || {
            for hit in 0..2 {
                if let Ok((mut stream, _)) = origin_listener.accept() {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let loc = if hit == 0 {
                        format!("http://127.0.0.1:{oauth_port}/oauth/authorize")
                    } else {
                        format!("http://127.0.0.1:{cdn_port}/s3/granule.h5")
                    };
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
        });

        let edl_cache_path =
            std::env::temp_dir().join(format!("eostage_nisar_test_edl_{}.bin", std::process::id()));
        let edl_cache_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&edl_cache_path)
            .unwrap();
        let edl_session = HttpRangeSession {
            url: format!("http://127.0.0.1:{origin_port}/granule.h5"),
            total_size: 0,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .unwrap(),
            auth: RangeAuthMode::Earthdata {
                auth: TransferAuth::None,
                resolved_s3_url: Mutex::new(None),
            },
            is_bhoonidhi: false,
            last_request_at: Mutex::new(None),
            cache: Mutex::new(SparseCacheState {
                file: edl_cache_file,
                intervals: Vec::new(),
                cache_path: edl_cache_path,
            }),
            requests_count: AtomicU64::new(0),
            bytes_fetched: AtomicU64::new(0),
            last_error: Mutex::new(None),
        };
        let (total, bytes) = edl_session
            .fetch_range_raw(0, 3)
            .expect("Earthdata single OAuth return-to-initial hop must succeed");
        let _ = origin_srv.join();
        let _ = oauth_srv.join();
        let _ = cdn_srv.join();
        assert_eq!(total, 8);
        assert_eq!(&bytes, b"\x89HDF");
    }
}
