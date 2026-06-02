use std::{future, sync::Arc, time::Duration};

use fs::FakeFs;
use gpui::TestAppContext;
use http_client::{AsyncBody, FakeHttpClient, HttpClient, Response};
use project::AgentRegistryStore;
use serde_json::json;

use crate::init_test;

#[gpui::test]
async fn registry_refresh_times_out_when_fetch_never_completes(cx: &mut TestAppContext) {
    init_test(cx);

    let fs = FakeFs::new(cx.executor());
    let http_client =
        FakeHttpClient::create(|_| future::pending::<anyhow::Result<Response<AsyncBody>>>())
            as Arc<dyn HttpClient>;

    let registry_store =
        cx.update(|cx| AgentRegistryStore::init_global(cx, fs.clone(), http_client));
    cx.run_until_parked();

    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();

    registry_store.update(cx, |store, _| {
        assert!(!store.is_fetching());
        assert!(
            store
                .fetch_error()
                .is_some_and(|error| error.contains("获取注册表超时(30秒)")),
            "期望获取注册表超时错误,实际得到 {:?}",
            store.fetch_error()
        );
    });
}

#[gpui::test]
async fn registry_refresh_does_not_block_sequentially_on_hung_icon_downloads(
    cx: &mut TestAppContext,
) {
    init_test(cx);

    let fs = FakeFs::new(cx.executor());
    let http_client = FakeHttpClient::create(|request| async move {
        if request.uri().to_string().contains("registry.json") {
            Ok(Response::builder()
                .status(200)
                .body(AsyncBody::from(
                    serde_json::to_string(&json!({
                        "version": "1",
                        "agents": [
                            {
                                "id": "slow-icon-a",
                                "name": "慢速图标 A",
                                "version": "1.0.0",
                                "description": "一个带有慢速图标的助手。",
                                "icon": "https://example.test/slow-icon-a.svg",
                                "distribution": {
                                    "npx": {
                                        "package": "slow-icon-a"
                                    }
                                }
                            },
                            {
                                "id": "slow-icon-b",
                                "name": "慢速图标 B",
                                "version": "1.0.0",
                                "description": "另一个带有慢速图标的助手。",
                                "icon": "https://example.test/slow-icon-b.svg",
                                "distribution": {
                                    "npx": {
                                        "package": "slow-icon-b"
                                    }
                                }
                            },
                            {
                                "id": "slow-icon-c",
                                "name": "慢速图标 C",
                                "version": "1.0.0",
                                "description": "第三个带有慢速图标的助手。",
                                "icon": "https://example.test/slow-icon-c.svg",
                                "distribution": {
                                    "npx": {
                                        "package": "slow-icon-c"
                                    }
                                }
                            }
                        ]
                    }))
                    .unwrap(),
                ))
                .unwrap())
        } else {
            future::pending::<anyhow::Result<Response<AsyncBody>>>().await
        }
    }) as Arc<dyn HttpClient>;

    let registry_store =
        cx.update(|cx| AgentRegistryStore::init_global(cx, fs.clone(), http_client));
    cx.run_until_parked();

    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();

    registry_store.update(cx, |store, _| {
        assert!(!store.is_fetching());
        assert_eq!(store.agents().len(), 3);
        assert_eq!(store.agents()[0].id().as_ref(), "slow-icon-a");
        assert_eq!(store.agents()[1].id().as_ref(), "slow-icon-b");
        assert_eq!(store.agents()[2].id().as_ref(), "slow-icon-c");
        assert_eq!(store.fetch_error(), None);
    });
}
