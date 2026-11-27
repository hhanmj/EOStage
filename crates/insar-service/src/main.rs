fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let result = match (
        arguments.next(),
        arguments.next(),
        arguments.next(),
        arguments.next(),
    ) {
        (None, None, None, None) => insar_service::run_stdio(),
        (Some(flag), Some(path), None, None) if flag == "--store-path" => {
            insar_service::run_stdio_with_store_path(std::path::PathBuf::from(path))
        }
        (Some(flag), Some(path), Some(enable), None)
            if flag == "--store-path" && enable == "--enable-rust-downloads" =>
        {
            insar_service::run_stdio_with_downloads(std::path::PathBuf::from(path))
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid sidecar arguments",
        )),
    };
    if result.is_err() {
        eprintln!("sidecar protocol I/O failure");
        std::process::exit(1);
    }
}
