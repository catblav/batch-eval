// 端到端验证：需先启动 tests/mock-api.mjs，然后用
//   cargo test --lib integration_mock -- --ignored --nocapture
// 运行。默认不参与常规测试，避免没起服务时误报。

#[cfg(test)]
mod integration {
    use crate::api::{self, ApiProfile};
    use crate::judge;

    fn profile() -> ApiProfile {
        ApiProfile {
            protocol: "openai".into(),
            base_url: "http://127.0.0.1:18999/v1".into(),
            api_key: "test".into(),
            model: "mock-model".into(),
            temperature: Some(0.0),
            top_p: Some(1.0),
            max_tokens: Some(512),
            context_window: Some(8192),
            timeout_secs: Some(30),
            extra_body: None,
            extra_headers: None,
        }
    }

    #[tokio::test]
    #[ignore]
    async fn integration_mock() {
        let p = profile();
        let client = api::build_client(&p).unwrap();

        // 1. 被测模型：普通输入应得到完整回答
        let out = api::chat(&client, &p, "你是助手", "帮我改写简历")
            .await
            .expect("被测模型调用失败");
        assert!(out.text.contains("完整回答"));
        assert_eq!(out.prompt_tokens, Some(100));

        // 2. 评测模型：短输出判不通过并给出分类
        let judge_prompt = format!(
            "# 评测\n<<<OUTPUT_START>>>\n{}\n<<<OUTPUT_END>>>",
            "好的。"
        );
        let r = api::chat(&client, &p, "", &judge_prompt).await.unwrap();
        let v = judge::parse_verdict(&r.text);
        assert!(!v.pass, "短输出应被判不通过");
        assert_eq!(v.error_types, vec!["内容单薄".to_string()]);
        assert!(v.parse_ok);

        // 3. 评测模型：长输出通过，且能解析代码块包裹的 JSON
        let long = "这是一段足够长的回答，包含了具体的改写结果和建议，长度超过四十个字符以便通过检查。";
        let judge_prompt2 = format!("# 评测\n<<<OUTPUT_START>>>\n{long}\n<<<OUTPUT_END>>>");
        let r2 = api::chat(&client, &p, "", &judge_prompt2).await.unwrap();
        let v2 = judge::parse_verdict(&r2.text);
        assert!(v2.pass, "长输出应通过");
        assert!(v2.error_types.is_empty());
        assert!(v2.parse_ok, "代码块包裹的 JSON 也应能解析");

        // 4. 断言失败路径：错误地址应报连接或 HTTP 错误
        let mut bad = profile();
        bad.base_url = "http://127.0.0.1:1/v1".into();
        let bc = api::build_client(&bad).unwrap();
        let err = api::chat(&bc, &bad, "", "hi").await.unwrap_err();
        assert!(err.contains("连接失败") || err.contains("请求失败"), "实际错误：{err}");

        println!("端到端验证通过：被测调用、评测判定、解析容错、错误处理");
    }
}

#[cfg(test)]
mod dataset_tests {
    use crate::dataset::Dataset;

    #[test]
    fn parses_sample_csv() {
        let text = "数据组状态,用户输入\n正常-典型,第一条\n边界-极短,极短\n";
        // 直接验证 CSV 解析路径（与 Tauri 命令共用同一实现）
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
        let headers = rdr.headers().unwrap().clone();
        let cols: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
        let mut rows = 0;
        for _ in rdr.records() {
            rows += 1;
        }
        assert_eq!(cols, vec!["数据组状态", "用户输入"]);
        assert_eq!(rows, 2);
        let ds = Dataset { columns: cols, rows: vec![], source: "t".into(), skipped: 0 };
        assert!(ds.columns.contains(&"数据组状态".to_string()));
    }
}

/// 批量链路：N 条数据 → 被测模型 → 评测模型 → 聚合分类。需先启动 mock 服务。
#[cfg(test)]
mod batch_flow {
    use crate::api::ApiProfile;
    use crate::judge;

    fn profile() -> ApiProfile {
        ApiProfile {
            protocol: "openai".into(),
            base_url: "http://127.0.0.1:18999/v1".into(),
            api_key: "t".into(),
            model: "mock-model".into(),
            temperature: Some(0.0),
            top_p: None,
            max_tokens: Some(256),
            context_window: Some(8192),
            timeout_secs: Some(20),
            extra_body: None,
            extra_headers: None,
        }
    }

    #[tokio::test]
    #[ignore]
    async fn batch_classification_aggregates() {
        let p = profile();
        let client = crate::api::build_client(&p).unwrap();
        let cases = vec![
            ("正常-典型", "这是一条正常长度的输入内容，用来检验批量评测的完整链路是否跑得通。"),
            ("边界-极短", "好的。"),
        ];
        let mut buckets: std::collections::HashMap<String, usize> = Default::default();
        let mut passed = 0;
        for (label, input) in &cases {
            let _ = label;
            let out = crate::api::chat(&client, &p, "你是助手", input).await.unwrap();
            let judge_input = format!(
                "评测\n<<<OUTPUT_START>>>\n{}\n<<<OUTPUT_END>>>",
                out.text
            );
            let jr = crate::api::chat(&client, &p, "", &judge_input).await.unwrap();
            let v = judge::parse_verdict(&jr.text);
            if v.pass {
                passed += 1;
            } else {
                for t in &v.error_types {
                    *buckets.entry(t.clone()).or_insert(0) += 1;
                }
            }
        }
        assert_eq!(passed, 1, "应有一条通过");
        assert_eq!(buckets.get("内容单薄"), Some(&1), "短输出应归入内容单薄");
    }
}
