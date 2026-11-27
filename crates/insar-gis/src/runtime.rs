use super::{NativeGisError, NativeGisErrorCode};
use libloading::Library;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

pub const SUPPORTED_GIS_DRIVERS: &[&str] = &["GTiff", "ENVI"];

type ProjContext = *mut c_void;
type ProjContextCreate = unsafe extern "C" fn() -> ProjContext;
type ProjContextDestroy = unsafe extern "C" fn(ProjContext);
type ProjContextSetEnableNetwork = unsafe extern "C" fn(ProjContext, c_int);
type ProjContextSetSearchPaths = unsafe extern "C" fn(ProjContext, c_int, *const *const c_char);

pub(crate) struct ProjContextGuard {
    context: ProjContext,
    destroy: ProjContextDestroy,
}

impl ProjContextGuard {
    pub(crate) fn raw(&self) -> ProjContext {
        self.context
    }
}

impl Drop for ProjContextGuard {
    fn drop(&mut self) {
        if !self.context.is_null() {
            unsafe { (self.destroy)(self.context) };
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFileManifest {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeGisRuntimeManifest {
    pub schema_version: u32,
    pub runtime_id: String,
    pub gdal_version: String,
    pub proj_version: String,
    pub gdal_library: String,
    pub proj_library: String,
    pub proj_data_dir: String,
    pub gdal_data_dir: String,
    pub proj_network: String,
    pub drivers: Vec<String>,
    #[serde(default)]
    pub files: Vec<RuntimeFileManifest>,
}

/// Cheap, immutable identity for the adjacent product runtime. Constructing
/// this value reads only `manifest.json`; it never walks the runtime tree,
/// hashes payload files, loads GDAL/PROJ, or opens a vertical-grid payload.
#[derive(Clone, Debug)]
pub struct NativeGisRuntimeIdentity {
    root: PathBuf,
    manifest: NativeGisRuntimeManifest,
    manifest_sha256: String,
}

impl NativeGisRuntimeIdentity {
    pub fn from_product_root(root: impl AsRef<Path>) -> Result<Self, NativeGisError> {
        let root = root.as_ref().to_path_buf();
        let raw = fs::read(root.join("manifest.json"))
            .map_err(|_| unavailable("product-owned native GIS manifest is missing"))?;
        let manifest: NativeGisRuntimeManifest = serde_json::from_slice(&raw)
            .map_err(|_| unavailable("product-owned native GIS manifest is invalid"))?;
        validate_manifest_identity(&manifest)?;
        let mut digest = Sha256::new();
        digest.update(&raw);
        Ok(Self {
            root,
            manifest,
            manifest_sha256: format!("{:x}", digest.finalize()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn runtime_id(&self) -> &str {
        &self.manifest.runtime_id
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn manifest(&self) -> &NativeGisRuntimeManifest {
        &self.manifest
    }
}

type GdalAllRegisterFn = unsafe extern "C" fn();
type CplSetConfigOptionFn = unsafe extern "C" fn(*const c_char, *const c_char);

struct LoadedGisLibraries {
    #[cfg(windows)]
    _dll_directory: ProductDllDirectory,
    gdal: Library,
    proj: Library,
}

type LoadedGisLibraryCache = HashMap<(PathBuf, PathBuf), Arc<LoadedGisLibraries>>;

static LOADED_GIS_LIBRARIES: OnceLock<Mutex<LoadedGisLibraryCache>> = OnceLock::new();
static GDAL_REGISTERED_HANDLES: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();

pub struct NativeGisRuntime {
    root: PathBuf,
    manifest: NativeGisRuntimeManifest,
    libraries: Arc<LoadedGisLibraries>,
}

impl NativeGisRuntime {
    pub fn load_identity(identity: &NativeGisRuntimeIdentity) -> Result<Self, NativeGisError> {
        let root = identity.root();
        let manifest = identity.manifest().clone();
        validate_manifest(root, &manifest)?;
        let gdal_path = root.join(&manifest.gdal_library);
        let proj_path = root.join(&manifest.proj_library);
        let canonical_key = (
            fs::canonicalize(&gdal_path).unwrap_or_else(|_| gdal_path.clone()),
            fs::canonicalize(&proj_path).unwrap_or_else(|_| proj_path.clone()),
        );
        let cache = LOADED_GIS_LIBRARIES.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let libraries = if let Some(existing) = guard.get(&canonical_key) {
            Arc::clone(existing)
        } else {
            #[cfg(windows)]
            let dll_directory = ProductDllDirectory::add(
                gdal_path
                    .parent()
                    .ok_or_else(|| unavailable("product-owned GDAL DLL directory is invalid"))?,
            )?;
            let gdal = load_library(&gdal_path, "GDAL")?;
            let proj = load_library(&proj_path, "PROJ")?;
            let loaded = Arc::new(LoadedGisLibraries {
                #[cfg(windows)]
                _dll_directory: dll_directory,
                gdal,
                proj,
            });
            guard.insert(canonical_key, Arc::clone(&loaded));
            loaded
        };
        drop(guard);
        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            libraries,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &NativeGisRuntimeManifest {
        &self.manifest
    }

    pub(crate) fn supports_driver(&self, driver: &str) -> bool {
        self.manifest
            .drivers
            .iter()
            .any(|candidate| candidate == driver)
    }

    pub(crate) unsafe fn gdal_symbol<T>(
        &self,
        name: &[u8],
    ) -> Result<libloading::Symbol<'_, T>, NativeGisError> {
        let symbol_name = String::from_utf8_lossy(name)
            .trim_end_matches('\0')
            .to_owned();
        self.libraries.gdal.get(name).map_err(|_| {
            unavailable(format!(
                "required GDAL runtime symbol is unavailable: {symbol_name}"
            ))
        })
    }

    pub(crate) unsafe fn proj_symbol<T>(
        &self,
        name: &[u8],
    ) -> Result<libloading::Symbol<'_, T>, NativeGisError> {
        let symbol_name = String::from_utf8_lossy(name)
            .trim_end_matches('\0')
            .to_owned();
        self.libraries.proj.get(name).map_err(|_| {
            unavailable(format!(
                "required PROJ runtime symbol is unavailable: {symbol_name}"
            ))
        })
    }

    pub(crate) fn configure_gdal_options(
        &self,
        options: &[(&str, &str)],
        proj_data: &std::ffi::CStr,
        gdal_data: &std::ffi::CStr,
    ) -> Result<(), NativeGisError> {
        let lock = GDAL_REGISTERED_HANDLES.get_or_init(|| Mutex::new(HashSet::new()));
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            let set_config = *self.gdal_symbol::<CplSetConfigOptionFn>(b"CPLSetConfigOption\0")?;
            for &(key, value) in options {
                let key_c = CString::new(key).expect("static config key");
                let val_c = CString::new(value).expect("static config value");
                set_config(key_c.as_ptr(), val_c.as_ptr());
            }
            let proj_key = CString::new("PROJ_DATA").expect("static config key");
            set_config(proj_key.as_ptr(), proj_data.as_ptr());
            let gdal_key = CString::new("GDAL_DATA").expect("static config key");
            set_config(gdal_key.as_ptr(), gdal_data.as_ptr());
        }
        Ok(())
    }

    pub(crate) fn ensure_gdal_registered(&self) -> Result<(), NativeGisError> {
        let lock = GDAL_REGISTERED_HANDLES.get_or_init(|| Mutex::new(HashSet::new()));
        let mut registered = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let handle_key = Arc::as_ptr(&self.libraries) as usize;
        if !registered.contains(&handle_key) {
            unsafe {
                let register = *self.gdal_symbol::<GdalAllRegisterFn>(b"GDALAllRegister\0")?;
                register();
            }
            registered.insert(handle_key);
        }
        Ok(())
    }

    pub(crate) fn configure_and_register_gdal(
        &self,
        options: &[(&str, &str)],
        proj_data: &std::ffi::CStr,
        gdal_data: &std::ffi::CStr,
    ) -> Result<(), NativeGisError> {
        let lock = GDAL_REGISTERED_HANDLES.get_or_init(|| Mutex::new(HashSet::new()));
        let mut registered = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            let set_config = *self.gdal_symbol::<CplSetConfigOptionFn>(b"CPLSetConfigOption\0")?;
            for &(key, value) in options {
                let key_c = CString::new(key).expect("static config key");
                let val_c = CString::new(value).expect("static config value");
                set_config(key_c.as_ptr(), val_c.as_ptr());
            }
            let proj_key = CString::new("PROJ_DATA").expect("static config key");
            set_config(proj_key.as_ptr(), proj_data.as_ptr());
            let gdal_key = CString::new("GDAL_DATA").expect("static config key");
            set_config(gdal_key.as_ptr(), gdal_data.as_ptr());
            let handle_key = Arc::as_ptr(&self.libraries) as usize;
            if !registered.contains(&handle_key) {
                let register = *self.gdal_symbol::<GdalAllRegisterFn>(b"GDALAllRegister\0")?;
                register();
                registered.insert(handle_key);
            }
        }
        Ok(())
    }

    pub(crate) fn create_proj_context(&self) -> Result<ProjContextGuard, NativeGisError> {
        self.create_proj_context_with_search_paths(&[])
    }

    pub(crate) fn create_proj_context_with_search_paths(
        &self,
        additional_paths: &[&Path],
    ) -> Result<ProjContextGuard, NativeGisError> {
        let mut search_paths = vec![self.root.join(&self.manifest.proj_data_dir)];
        search_paths.extend(additional_paths.iter().map(|path| (*path).to_path_buf()));
        let search_path_strings = search_paths
            .iter()
            .map(|path| {
                CString::new(path.to_string_lossy().as_bytes())
                    .map_err(|_| unavailable("PROJ 搜索路径编码无效"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let search_path_ptrs = search_path_strings
            .iter()
            .map(|path| path.as_ptr())
            .collect::<Vec<_>>();
        unsafe {
            let create = *self.proj_symbol::<ProjContextCreate>(b"proj_context_create\0")?;
            let destroy = *self.proj_symbol::<ProjContextDestroy>(b"proj_context_destroy\0")?;
            let set_network = *self
                .proj_symbol::<ProjContextSetEnableNetwork>(b"proj_context_set_enable_network\0")?;
            let set_search_paths = *self
                .proj_symbol::<ProjContextSetSearchPaths>(b"proj_context_set_search_paths\0")?;
            let context = create();
            if context.is_null() {
                return Err(unavailable(
                    "product-owned PROJ context could not be created",
                ));
            }
            set_search_paths(
                context,
                search_path_ptrs.len() as c_int,
                search_path_ptrs.as_ptr(),
            );
            set_network(context, 0);
            Ok(ProjContextGuard { context, destroy })
        }
    }
}

#[cfg(windows)]
struct ProductDllDirectory {
    // The Windows API treats this as an opaque cookie.  Store its numeric
    // value so the runtime can be shared by the Application Service's
    // Send+Sync operation port without exposing a raw pointer to Rust's
    // auto-trait analysis; it is cast back only for RemoveDllDirectory.
    cookie: usize,
}

#[cfg(windows)]
impl ProductDllDirectory {
    fn add(path: &Path) -> Result<Self, NativeGisError> {
        if !path.is_absolute() {
            return Err(unavailable(
                "product-owned native GIS DLL directory must be absolute",
            ));
        }
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let cookie = unsafe { AddDllDirectory(wide.as_ptr()) };
        if cookie.is_null() {
            return Err(unavailable(
                "product-owned native GIS DLL directory could not be registered",
            ));
        }
        Ok(Self {
            cookie: cookie as usize,
        })
    }
}

#[cfg(windows)]
impl Drop for ProductDllDirectory {
    fn drop(&mut self) {
        unsafe {
            RemoveDllDirectory(self.cookie as *mut c_void);
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn AddDllDirectory(new_directory: *const u16) -> *mut c_void;
    fn RemoveDllDirectory(cookie: *mut c_void) -> i32;
}

fn load_library(path: &Path, name: &str) -> Result<Library, NativeGisError> {
    let library = unsafe { load_library_with_product_search_path(path) }.map_err(|error| {
        unavailable(format!(
            "product-owned {name} runtime could not be loaded from {}: {error:?}",
            path.display()
        ))
    })?;
    Ok(library)
}

#[cfg(windows)]
unsafe fn load_library_with_product_search_path(path: &Path) -> Result<Library, libloading::Error> {
    use libloading::os::windows::{
        Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    };

    // The product runtime is a private DLL closure.  Ask the Windows loader to
    // resolve dependencies from the loaded DLL's own directory and the normal
    // system directories only; do not consult the process PATH, Conda, QGIS,
    // or an inherited GDAL plugin directory.
    let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
    WindowsLibrary::load_with_flags(path, flags).map(Into::into)
}

#[cfg(not(windows))]
unsafe fn load_library_with_product_search_path(path: &Path) -> Result<Library, libloading::Error> {
    Library::new(path)
}

pub(crate) fn validate_manifest(
    root: &Path,
    manifest: &NativeGisRuntimeManifest,
) -> Result<(), NativeGisError> {
    validate_manifest_identity(manifest)?;
    let proj_db = root.join(&manifest.proj_data_dir).join("proj.db");
    if !proj_db.is_file() {
        return Err(unavailable("product-owned PROJ database is missing"));
    }
    for directory in [&manifest.proj_data_dir, &manifest.gdal_data_dir] {
        if !root.join(directory).is_dir() {
            return Err(unavailable("native GIS data directory is missing"));
        }
    }
    let mut declared_paths = BTreeSet::<String>::new();
    let mut base_paths = BTreeSet::<String>::new();
    for file in &manifest.files {
        if !declared_paths.insert(file.relative_path.clone()) {
            return Err(unavailable(
                "native GIS runtime manifest contains duplicate files",
            ));
        }
        if is_vertical_grid_path(&file.relative_path) {
            continue;
        }
        base_paths.insert(file.relative_path.clone());
        let path = root.join(&file.relative_path);
        let metadata =
            fs::metadata(&path).map_err(|_| unavailable("native GIS runtime file is missing"))?;
        if !metadata.is_file() || metadata.len() != file.bytes {
            return Err(unavailable(
                "native GIS runtime file size does not match its manifest",
            ));
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(&file.sha256) {
            return Err(unavailable(
                "native GIS runtime file hash does not match its manifest",
            ));
        }
    }
    let mut actual_paths = BTreeSet::new();
    collect_runtime_files(root, root, &mut actual_paths)?;
    actual_paths.remove("manifest.json");
    if actual_paths != base_paths {
        return Err(unavailable(
            "native GIS runtime files do not exactly match the manifest",
        ));
    }
    Ok(())
}

fn validate_manifest_identity(manifest: &NativeGisRuntimeManifest) -> Result<(), NativeGisError> {
    if manifest.schema_version != 1
        || manifest.runtime_id.trim().is_empty()
        || manifest.gdal_version.trim().is_empty()
        || manifest.proj_version.trim().is_empty()
        || manifest.proj_network != "disabled"
    {
        return Err(unavailable(
            "native GIS runtime identity or explicit PROJ network policy is invalid",
        ));
    }
    let allowed = SUPPORTED_GIS_DRIVERS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let drivers = manifest
        .drivers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if drivers != allowed || manifest.drivers.len() != allowed.len() {
        return Err(error(
            NativeGisErrorCode::GisDriverUnavailable,
            "native GIS runtime driver list must be exactly GTiff and ENVI",
        ));
    }
    for path in [
        &manifest.gdal_library,
        &manifest.proj_library,
        &manifest.proj_data_dir,
        &manifest.gdal_data_dir,
    ] {
        if !is_safe_relative_path(path) {
            return Err(unavailable(
                "native GIS runtime contains an unsafe relative path",
            ));
        }
    }
    let mut declared_paths = BTreeSet::new();
    for file in &manifest.files {
        if !is_safe_relative_path(&file.relative_path)
            || file.sha256.len() != 64
            || !file
                .sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(unavailable("native GIS runtime file path is unsafe"));
        }
        if !declared_paths.insert(file.relative_path.clone()) {
            return Err(unavailable(
                "native GIS runtime manifest contains duplicate files",
            ));
        }
    }
    Ok(())
}

fn collect_runtime_files(
    root: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), NativeGisError> {
    for entry in fs::read_dir(current)
        .map_err(|_| unavailable("native GIS runtime directory cannot be read"))?
    {
        let entry = entry.map_err(|_| unavailable("native GIS runtime entry cannot be read"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| unavailable("native GIS runtime entry type cannot be read"))?;
        if file_type.is_dir() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| unavailable("native GIS runtime directory is outside its root"))?
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "vertical-grids" {
                continue;
            }
            collect_runtime_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| unavailable("native GIS runtime file is outside its root"))?
                .to_string_lossy()
                .replace('\\', "/");
            if !is_safe_relative_path(&relative) {
                return Err(unavailable("native GIS runtime file path is unsafe"));
            }
            files.insert(relative);
        } else {
            return Err(unavailable(
                "native GIS runtime contains a non-regular filesystem entry",
            ));
        }
    }
    Ok(())
}

fn is_safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::ParentDir
            )
        })
}

fn sha256_file(path: &Path) -> Result<String, NativeGisError> {
    let bytes =
        fs::read(path).map_err(|_| unavailable("native GIS runtime file cannot be read"))?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    Ok(format!("{:x}", digest.finalize()))
}

fn unavailable(reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(NativeGisErrorCode::GisEngineUnavailable, reason)
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn is_vertical_grid_path(value: &str) -> bool {
    value == "vertical-grids" || value.starts_with("vertical-grids/")
}

/// Subprocess scratch directory budget and timeout policy (adapted from
/// `opengeos/RustGIS` `gdal_compat.rs`, Apache-2.0; see `THIRD-PARTY-NOTICES.md`).
/// Isolates external GIS subprocesses inside a dedicated scratch directory
/// (`CPL_TMPDIR`, `TMPDIR`, `TMP`, `TEMP`) and enforces byte-usage, file-count,
/// and wall-time quotas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubprocessScratchBudget {
    pub max_scratch_bytes: u64,
    pub max_scratch_files: usize,
    pub timeout: std::time::Duration,
    pub poll_interval: std::time::Duration,
}

impl Default for SubprocessScratchBudget {
    fn default() -> Self {
        Self {
            max_scratch_bytes: 512 * 1024 * 1024,
            max_scratch_files: 1_024,
            timeout: std::time::Duration::from_secs(60),
            poll_interval: std::time::Duration::from_millis(25),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubprocessScratchOutcome {
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub peak_scratch_bytes: u64,
    pub peak_scratch_files: usize,
}

pub fn measure_directory_usage(root: &Path) -> Result<(u64, usize), NativeGisError> {
    fn walk(current: &Path, depth: usize) -> Result<(u64, usize), NativeGisError> {
        if depth > 16 {
            return Err(unavailable(
                "subprocess scratch directory exceeded maximum recursion depth",
            ));
        }
        let mut total_bytes = 0u64;
        let mut total_files = 0usize;
        let entries = fs::read_dir(current).map_err(|_| {
            unavailable("subprocess scratch directory cannot be read during quota check")
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                unavailable("subprocess scratch directory entry cannot be inspected")
            })?;
            let file_type = entry
                .file_type()
                .map_err(|_| unavailable("subprocess scratch directory entry type unreadable"))?;
            if file_type.is_symlink() {
                return Err(unavailable(
                    "subprocess scratch directory must not contain symbolic links",
                ));
            }
            if file_type.is_dir() {
                let (sub_bytes, sub_files) = walk(&entry.path(), depth + 1)?;
                total_bytes = total_bytes.saturating_add(sub_bytes);
                total_files = total_files.saturating_add(sub_files);
            } else {
                let metadata = entry
                    .metadata()
                    .map_err(|_| unavailable("subprocess scratch file metadata unreadable"))?;
                total_bytes = total_bytes.saturating_add(metadata.len());
                total_files = total_files.saturating_add(1);
            }
        }
        Ok((total_bytes, total_files))
    }

    let root_meta = fs::symlink_metadata(root)
        .map_err(|_| unavailable("subprocess scratch root directory is missing"))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(unavailable(
            "subprocess scratch root must be a real directory",
        ));
    }
    walk(root, 0)
}

impl SubprocessScratchBudget {
    pub fn execute_command(
        &self,
        command: &mut std::process::Command,
        scratch_dir: &Path,
    ) -> Result<SubprocessScratchOutcome, NativeGisError> {
        let _ = measure_directory_usage(scratch_dir)?;
        command
            .env("CPL_TMPDIR", scratch_dir)
            .env("TMPDIR", scratch_dir)
            .env("TMP", scratch_dir)
            .env("TEMP", scratch_dir)
            .env("PROJ_NETWORK", "OFF")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|error| unavailable(format!("failed to spawn GIS subprocess: {error}")))?;

        let stdout_handle = child.stdout.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
                buf
            })
        });
        let stderr_handle = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
                buf
            })
        });

        let started = std::time::Instant::now();
        let mut peak_bytes = 0u64;
        let mut peak_files = 0usize;

        let status = loop {
            let (used_bytes, used_files) = match measure_directory_usage(scratch_dir) {
                Ok(usage) => usage,
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err);
                }
            };
            peak_bytes = peak_bytes.max(used_bytes);
            peak_files = peak_files.max(used_files);

            if used_bytes > self.max_scratch_bytes {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable(format!(
                    "subprocess scratch byte quota exceeded ({used_bytes} > {})",
                    self.max_scratch_bytes
                )));
            }
            if used_files > self.max_scratch_files {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable(format!(
                    "subprocess scratch file count quota exceeded ({used_files} > {})",
                    self.max_scratch_files
                )));
            }
            if started.elapsed() > self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable(format!(
                    "subprocess exceeded wall-time budget ({:?})",
                    self.timeout
                )));
            }

            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(self.poll_interval),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(unavailable(format!(
                        "failed to poll GIS subprocess status: {error}"
                    )));
                }
            }
        };

        let (final_bytes, final_files) = measure_directory_usage(scratch_dir)?;
        peak_bytes = peak_bytes.max(final_bytes);
        peak_files = peak_files.max(final_files);
        if peak_bytes > self.max_scratch_bytes {
            return Err(unavailable(format!(
                "subprocess scratch byte quota exceeded ({peak_bytes} > {})",
                self.max_scratch_bytes
            )));
        }
        if peak_files > self.max_scratch_files {
            return Err(unavailable(format!(
                "subprocess scratch file count quota exceeded ({peak_files} > {})",
                self.max_scratch_files
            )));
        }

        let stdout = stdout_handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        let stderr = stderr_handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();

        Ok(SubprocessScratchOutcome {
            status_code: status.code(),
            stdout,
            stderr,
            peak_scratch_bytes: peak_bytes,
            peak_scratch_files: peak_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        measure_directory_usage, validate_manifest, NativeGisRuntimeManifest, RuntimeFileManifest,
        SubprocessScratchBudget,
    };
    use crate::NativeGisErrorCode;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::time::Duration;

    fn manifest(drivers: Vec<String>) -> NativeGisRuntimeManifest {
        NativeGisRuntimeManifest {
            schema_version: 1,
            runtime_id: "test-native-gis".to_owned(),
            gdal_version: "3.10.3".to_owned(),
            proj_version: "9.5.1".to_owned(),
            gdal_library: "bin/gdal.dll".to_owned(),
            proj_library: "bin/proj.dll".to_owned(),
            proj_data_dir: "share/proj".to_owned(),
            gdal_data_dir: "share/gdal".to_owned(),
            proj_network: "disabled".to_owned(),
            drivers,
            files: Vec::new(),
        }
    }

    #[test]
    fn runtime_rejects_unbounded_driver_before_loading_any_dll() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-gis-manifest-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/proj")).expect("test directory");
        fs::create_dir_all(root.join("share/gdal")).expect("test directory");
        fs::write(root.join("share/proj/proj.db"), b"fixture").expect("proj fixture");
        let error = validate_manifest(
            &root,
            &manifest(vec!["GTiff".to_owned(), "PostgreSQL".to_owned()]),
        )
        .expect_err("unbounded driver must fail closed");
        assert_eq!(error.code(), NativeGisErrorCode::GisDriverUnavailable);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_rejects_rust_owned_vector_driver_from_native_manifest() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-gis-raster-driver-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/proj")).expect("test directory");
        fs::create_dir_all(root.join("share/gdal")).expect("test directory");
        fs::write(root.join("share/proj/proj.db"), b"fixture").expect("proj fixture");
        let error = validate_manifest(
            &root,
            &manifest(vec!["GTiff".to_owned(), "ESRI Shapefile".to_owned()]),
        )
        .expect_err("vector driver must remain outside the native raster contract");
        assert_eq!(error.code(), NativeGisErrorCode::GisDriverUnavailable);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_requires_both_bounded_raster_drivers() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-gis-driver-completeness-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/proj")).expect("test directory");
        fs::create_dir_all(root.join("share/gdal")).expect("test directory");
        fs::write(root.join("share/proj/proj.db"), b"fixture").expect("proj fixture");
        let error = validate_manifest(&root, &manifest(vec!["GTiff".to_owned()]))
            .expect_err("the product runtime must advertise both bounded raster drivers");
        assert_eq!(error.code(), NativeGisErrorCode::GisDriverUnavailable);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_rejects_an_unlisted_file_from_the_product_closure() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-gis-manifest-files-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/proj")).expect("test directory");
        fs::create_dir_all(root.join("share/gdal")).expect("test directory");
        fs::write(root.join("share/proj/proj.db"), b"fixture").expect("proj fixture");
        fs::write(root.join("unexpected.dll"), b"not in the closure").expect("extra file");
        let error = validate_manifest(
            &root,
            &manifest(vec!["GTiff".to_owned(), "ENVI".to_owned()]),
        )
        .expect_err("unlisted runtime files must fail closed");
        assert_eq!(error.code(), NativeGisErrorCode::GisEngineUnavailable);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn base_runtime_validation_does_not_read_vertical_grid_payloads() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-gis-capability-specific-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/proj")).expect("PROJ test directory");
        fs::create_dir_all(root.join("share/gdal")).expect("GDAL test directory");
        let proj_db = b"base-runtime-fixture";
        fs::write(root.join("share/proj/proj.db"), proj_db).expect("PROJ fixture");
        let mut digest = Sha256::new();
        digest.update(proj_db);
        let mut runtime = manifest(vec!["GTiff".to_owned(), "ENVI".to_owned()]);
        runtime.files = vec![
            RuntimeFileManifest {
                relative_path: "share/proj/proj.db".to_owned(),
                bytes: proj_db.len() as u64,
                sha256: format!("{:x}", digest.finalize()),
            },
            RuntimeFileManifest {
                relative_path: "vertical-grids/egm96/grid.tif".to_owned(),
                bytes: 2_710_815,
                sha256: "0".repeat(64),
            },
            RuntimeFileManifest {
                relative_path: "vertical-grids/egm2008/grid.tif".to_owned(),
                bytes: 80_585_622,
                sha256: "0".repeat(64),
            },
        ];

        validate_manifest(&root, &runtime)
            .expect("base runtime verification must defer both vertical-grid payloads");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subprocess_scratch_budget_enforces_byte_and_file_quotas() {
        let scratch = std::env::temp_dir().join(format!(
            "insar-studio-gis-scratch-budget-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&scratch).expect("scratch dir");
        fs::write(scratch.join("a.tmp"), vec![0u8; 128]).expect("scratch file a");
        fs::write(scratch.join("b.tmp"), vec![0u8; 256]).expect("scratch file b");

        let (bytes, files) = measure_directory_usage(&scratch).expect("directory usage");
        assert_eq!(bytes, 384);
        assert_eq!(files, 2);

        let tight_budget = SubprocessScratchBudget {
            max_scratch_bytes: 200,
            max_scratch_files: 10,
            timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(10),
        };
        #[cfg(unix)]
        let mut cmd = {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg("printf ok");
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.arg("/C").arg("echo ok");
            c
        };
        let err = tight_budget
            .execute_command(&mut cmd, &scratch)
            .expect_err("scratch usage exceeding byte quota must fail closed");
        assert_eq!(err.code(), NativeGisErrorCode::GisEngineUnavailable);
        let _ = fs::remove_dir_all(scratch);
    }
}
