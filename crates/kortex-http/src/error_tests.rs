use super::*;
#[tokio::test]
async fn encoded_errors_fit_minimum_budget_and_map_shared_exit_codes() {
    let id = hash("test request");
    for (code, exit, status) in [
        ("QUERY_TIMEOUT", 9, StatusCode::GATEWAY_TIMEOUT),
        ("QUERY_CANCELLED", 9, StatusCode::GATEWAY_TIMEOUT),
        ("INDEX_NOT_FOUND", 3, StatusCode::NOT_FOUND),
        ("INVALID_INPUT", 2, StatusCode::BAD_REQUEST),
        ("INDEX_CORRUPT", 6, StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = error_response(Error::new(code, "🧪\"\n".repeat(8000), exit), &id);
        assert_eq!(response.status(), status);
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(bytes.len() <= 1024);
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], code);
        assert_eq!(body["error"]["exit_code"], exit);
        assert_eq!(body["request_id"], id);
        if exit == 6 {
            assert_eq!(body["error"]["message"], "local service operation failed");
        }
    }
}
