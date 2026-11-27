#[test]
fn burst_task_mutations_wake_the_scheduler_once_after_store_success() {
    let source = include_str!("../src/lib.rs");
    for (method, next_method) in [
        (
            "pub fn create_burst_asset_task",
            "pub fn append_burst_asset_task",
        ),
        (
            "pub fn append_burst_asset_task",
            "pub fn create_historical_burst_asset_task",
        ),
        (
            "pub fn create_historical_burst_asset_task",
            "pub fn bind_region_aoi",
        ),
    ] {
        let start = source.find(method).expect("Burst planner method exists");
        let end = source[start..]
            .find(next_method)
            .map(|offset| start + offset)
            .expect("next method bounds Burst planner");
        let body = &source[start..end];
        assert_eq!(
            body.matches(".wake();").count(),
            1,
            "{method} must wake exactly once after its Store transaction succeeds"
        );
        let mutation = body
            .rfind(".map_err(ApplicationServiceError::from)?;")
            .expect("Store mutation is guarded");
        let wake = body.find(".wake();").expect("scheduler wake exists");
        assert!(
            mutation < wake,
            "{method} must never wake on failed or stale mutation"
        );
    }
}
