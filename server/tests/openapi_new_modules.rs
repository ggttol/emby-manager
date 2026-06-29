use emby_manager::openapi::ApiDoc;
use utoipa::OpenApi;

#[test]
fn openapi_registers_dedup_wizard_and_zhuigeng_modules() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    for path in [
        "/api/v2/dedup/duplicates",
        "/api/v2/dedup/execute",
        "/api/v2/dedup/execute-batch",
        "/api/v2/dedup/replace",
        "/api/v2/dedup/replace-batch",
        "/api/v2/dedup/auto-all",
        "/api/v2/catalog/transfer/execute",
        "/api/v2/c115/test-candidate",
        "/api/v2/posters/refresh-series",
        "/api/v2/wizard/add-new",
        "/api/v2/zhuigeng",
        "/api/v2/zhuigeng/scan-airing",
        "/api/v2/zhuigeng/gaps-summary",
        "/api/v2/libraries/items",
        "/api/v2/gaps/series",
    ] {
        assert!(
            paths.contains_key(path),
            "{path} missing from OpenAPI paths"
        );
    }

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    for schema in [
        "DedupAnalysisResponse",
        "DedupExecuteRequest",
        "DedupExecuteBatchRequest",
        "ReplaceExecuteResponse",
        "ReplaceBatchRequest",
        "DedupAutoAllResponse",
        "DashboardTodoResponse",
        "CatalogTransferExecuteRequest",
        "C115TestCandidateRequest",
        "RefreshSeriesRequest",
        "AddNewRequest",
        "AddNewReport",
        "ZhuigengStatusResponse",
        "ZhuigengGapsSummaryResponse",
        "SeriesGapsResponse",
    ] {
        assert!(
            schemas.contains_key(schema),
            "{schema} missing from OpenAPI schemas"
        );
    }
}
