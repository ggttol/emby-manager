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
        "/api/v2/catalog/remote-search",
        "/api/v2/catalog/library-context",
        "/api/v2/c115/test-candidate",
        "/api/v2/dashboard/smart-actions",
        "/api/v2/smart-actions",
        "/api/v2/smart-actions/summary",
        "/api/v2/smart-actions/policies",
        "/api/v2/smart-actions/policies/{key}",
        "/api/v2/smart-actions/execute-batch",
        "/api/v2/smart-actions/inspect",
        "/api/v2/smart-actions/refresh",
        "/api/v2/smart-actions/from-task/{task_id}",
        "/api/v2/smart-actions/from-next-action",
        "/api/v2/smart-actions/{id}",
        "/api/v2/smart-actions/{id}/execute",
        "/api/v2/smart-actions/{id}/dismiss",
        "/api/v2/smart-actions/{id}/verify",
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
        "DashboardSmartActionsResponse",
        "DashboardSmartAction",
        "SmartAction",
        "SmartActionsListResponse",
        "SmartActionsSummaryResponse",
        "SmartActionDetailResponse",
        "SmartActionExecuteRequest",
        "SmartActionExecuteResponse",
        "SmartActionExecuteBatchRequest",
        "SmartActionExecuteBatchResponse",
        "SmartActionFromTaskResponse",
        "SmartActionFromNextActionRequest",
        "SmartActionFromNextActionResponse",
        "SmartActionInspectRequest",
        "SmartActionInspectResponse",
        "SmartActionDismissRequest",
        "SmartActionDismissResponse",
        "SmartActionVerifyResponse",
        "SmartActionTaskResult",
        "SmartNextAction",
        "SmartNextActionSubject",
        "SmartActionPolicy",
        "SmartActionPoliciesResponse",
        "SmartActionPolicyUpdateRequest",
        "SmartActionPolicyUpdateResponse",
        "SmartEvidence",
        "SmartExecutionPlan",
        "SmartRisk",
        "SmartPolicyDecision",
        "CatalogTransferExecuteRequest",
        "CatalogRemoteSearchResponse",
        "CatalogLibraryContextResponse",
        "CatalogResourceRecommendation",
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

    let smart_task_result = &schemas["SmartActionTaskResult"];
    assert_eq!(
        smart_task_result["properties"]["next_actions"]["items"]["$ref"],
        "#/components/schemas/SmartNextAction"
    );
}
