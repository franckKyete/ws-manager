use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use ws_core::hub::HubClient;
use ws_core::{ConfigLoader, WorkspaceManager};

static TEST_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct MockRoute {
    method: String,
    path_prefix: String,
    status_code: u16,
    body: String,
}

struct MockServer {
    url: String,
    received_requests: Arc<Mutex<Vec<(String, String, String)>>>, // (method, path_and_query, body)
    shutdown: Arc<AtomicBool>,
}

impl MockServer {
    fn start(routes: Vec<MockRoute>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://127.0.0.1:{}", port);
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = vec![0u8; 16384];
                        if let Ok(n) = stream.read(&mut buf) {
                            if n > 0 {
                                let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                                let lines: Vec<&str> = req_str.split("\r\n").collect();
                                if !lines.is_empty() {
                                    let req_line: Vec<&str> = lines[0].split_whitespace().collect();
                                    if req_line.len() >= 2 {
                                        let method = req_line[0].to_string();
                                        let uri = req_line[1].to_string();
                                        let body = if let Some(pos) = req_str.find("\r\n\r\n") {
                                            req_str[pos + 4..].to_string()
                                        } else {
                                            String::new()
                                        };

                                        received_clone.lock().unwrap().push((
                                            method.clone(),
                                            uri.clone(),
                                            body.clone(),
                                        ));

                                        let mut matched = None;
                                        for r in &routes {
                                            if r.method == method && uri.starts_with(&r.path_prefix)
                                            {
                                                matched = Some(r.clone());
                                                break;
                                            }
                                        }

                                        if let Some(r) = matched {
                                            let resp = format!(
                                                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                                r.status_code,
                                                r.body.len(),
                                                r.body
                                            );
                                            let _ = stream.write_all(resp.as_bytes());
                                        } else {
                                            let not_found =
                                                "{\"status\":\"error\",\"message\":\"Not found\"}";
                                            let resp = format!(
                                                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                                not_found.len(),
                                                not_found
                                            );
                                            let _ = stream.write_all(resp.as_bytes());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        MockServer {
            url,
            received_requests: received,
            shutdown,
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

#[test]
fn test_e2e_hub_secrets_flow() {
    let routes = vec![
        MockRoute {
            method: "PUT".to_string(),
            path_prefix: "/api/v1/projects/myorg/myproject/secrets".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "status": "success",
                "message": "Secret 'API_KEY' saved."
            })
            .to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/myorg/myproject/secrets/API_KEY".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "status": "success",
                "data": { "key": "API_KEY", "value": "supersecret123" }
            })
            .to_string(),
        },
        MockRoute {
            method: "DELETE".to_string(),
            path_prefix: "/api/v1/projects/myorg/myproject/secrets/API_KEY".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "status": "success",
                "data": { "deleted": true }
            })
            .to_string(),
        },
    ];

    let server = MockServer::start(routes);
    let client = HubClient::new(Some(&server.url), Some("test_token"), None);

    // 1. Set secret (verify PUT method and body)
    let set_res = client.set_secret(
        "myorg",
        "myproject",
        "API_KEY",
        "supersecret123",
        Some("backend"),
    );
    assert!(set_res.is_ok(), "set_secret failed: {:?}", set_res.err());

    // 2. Get secret (verify query params repo and repoName)
    let val = client
        .get_secret("myorg", "myproject", "API_KEY", Some("backend"))
        .unwrap();
    assert_eq!(val, "supersecret123");

    // 3. Delete secret
    let del_ok = client
        .delete_secret("myorg", "myproject", "API_KEY", Some("backend"))
        .unwrap();
    assert!(del_ok);

    // Verify all requests received by mock server
    let reqs = server.received_requests.lock().unwrap();
    assert_eq!(reqs.len(), 3);

    // Check PUT
    assert_eq!(reqs[0].0, "PUT");
    assert!(reqs[0]
        .1
        .contains("/api/v1/projects/myorg/myproject/secrets"));
    assert!(reqs[0].2.contains("\"key\":\"API_KEY\""));
    assert!(reqs[0].2.contains("\"repoName\":\"backend\""));

    // Check GET
    assert_eq!(reqs[1].0, "GET");
    assert!(reqs[1].1.contains("repo=backend"));
    assert!(reqs[1].1.contains("repoName=backend"));

    // Check DELETE
    assert_eq!(reqs[2].0, "DELETE");
    assert!(reqs[2].1.contains("repo=backend"));
    assert!(reqs[2].1.contains("repoName=backend"));
}

#[test]
fn test_e2e_hub_workspace_states_flow() {
    let routes = vec![
        MockRoute {
            method: "POST".to_string(),
            path_prefix: "/api/v1/workspaces/myorg/myproject/states".to_string(),
            status_code: 201,
            body: serde_json::json!({
                "status": "success",
                "message": "Saved state for 'feat-login'"
            })
            .to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/workspaces/myorg/myproject/states/feat-login".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "status": "success",
                "data": {
                    "workspaceName": "feat-login",
                    "stateJson": serde_json::json!({
                        "name": "feat-login",
                        "repositories": {}
                    }).to_string()
                }
            })
            .to_string(),
        },
    ];

    let server = MockServer::start(routes);
    let client = HubClient::new(Some(&server.url), Some("test_token"), None);

    // 1. Save workspace state
    let state_payload = serde_json::json!({
        "name": "feat-login",
        "repositories": {}
    });
    let save_res = client.save_workspace_state("myorg", "myproject", "feat-login", &state_payload);
    assert!(
        save_res.is_ok(),
        "save_workspace_state failed: {:?}",
        save_res.err()
    );

    // 2. Get workspace state and verify stateJson is unpacked into a JSON object
    let restored = client
        .get_workspace_state("myorg", "myproject", "feat-login")
        .unwrap();
    assert_eq!(
        restored.get("name").and_then(|n| n.as_str()),
        Some("feat-login")
    );

    // Verify requests
    let reqs = server.received_requests.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].0, "POST");
    assert!(reqs[0]
        .1
        .contains("/api/v1/workspaces/myorg/myproject/states"));
    assert!(reqs[0].2.contains("\"workspaceName\":\"feat-login\""));
    assert!(reqs[0].2.contains("\"stateJson\":"));
}

#[test]
fn test_e2e_clone_from_hub_wshub_format() {
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let upstream = tempdir().unwrap();
    let upstream_bare = upstream.path().join("backend.git");
    let status = Command::new("git")
        .args(["init", "--bare", upstream_bare.to_str().unwrap()])
        .status()
        .expect("git init failed");
    assert!(status.success());

    let blueprint_content = format!(
        "repositories:\n  backend:\n    url: {}\n    bare: bares/backend.git\n    checkout: backend\n",
        upstream_bare.display()
    );

    let routes = vec![
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "project": {
                    "id": "proj_123",
                    "namespace": "kyete",
                    "name": "projecti-ws",
                    "description": "Polyrepo workspace",
                    "isPrivate": true
                },
                "latestRevision": {
                    "id": "rev_7",
                    "version": 7,
                    "blueprintYaml": blueprint_content,
                    "changelog": "Initial sync",
                    "createdAt": "2026-10-10T00:00:00Z"
                }
            })
            .to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws/files".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws/secrets".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
    ];

    let server = MockServer::start(routes);
    std::env::set_var("WS_HUB_URL", &server.url);
    std::env::set_var("WS_HUB_TOKEN", "test_token_123");

    let dest = tempdir().unwrap();
    let clone_target = dest.path().join("cloned_ws");

    let cfg = ConfigLoader::load_config(None, None, true).unwrap();
    let manager = WorkspaceManager::new(cfg, None);
    let res = manager.clone_from_hub("kyete/projecti-ws", Some(&clone_target));
    assert!(res.is_ok(), "clone_from_hub failed: {:?}", res.err());

    // Verify repositories.yml exists with the blueprint content
    let repo_yml = clone_target.join("repositories.yml");
    assert!(repo_yml.exists());
    let content = std::fs::read_to_string(&repo_yml).unwrap();
    assert_eq!(content, blueprint_content);

    // Verify bares and workspaces directories exist
    assert!(clone_target.join("bares").is_dir());
    assert!(clone_target.join("workspaces").is_dir());
    assert!(clone_target.join("bares/backend.git").exists());
}

#[test]
fn test_e2e_clone_from_hub_legacy_fallback_format() {
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let upstream = tempdir().unwrap();
    let upstream_bare = upstream.path().join("frontend.git");
    let status = Command::new("git")
        .args(["init", "--bare", upstream_bare.to_str().unwrap()])
        .status()
        .expect("git init failed");
    assert!(status.success());

    let blueprint_content = format!(
        "repositories:\n  frontend:\n    url: {}\n    bare: bares/frontend.git\n    checkout: frontend\n",
        upstream_bare.display()
    );

    let routes = vec![
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/legacy-ws".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "blueprint": {
                    "yaml": blueprint_content
                }
            })
            .to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/legacy-ws/files".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/legacy-ws/secrets".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
    ];

    let server = MockServer::start(routes);
    std::env::set_var("WS_HUB_URL", &server.url);
    std::env::set_var("WS_HUB_TOKEN", "test_token_123");

    let dest = tempdir().unwrap();
    let clone_target = dest.path().join("cloned_legacy");

    let cfg = ConfigLoader::load_config(None, None, true).unwrap();
    let manager = WorkspaceManager::new(cfg, None);
    let res = manager.clone_from_hub("kyete/legacy-ws", Some(&clone_target));
    assert!(res.is_ok(), "legacy clone_from_hub failed: {:?}", res.err());

    let repo_yml = clone_target.join("repositories.yml");
    assert!(repo_yml.exists());
    let content = std::fs::read_to_string(&repo_yml).unwrap();
    assert_eq!(content, blueprint_content);
    assert!(clone_target.join("bares/frontend.git").exists());
}

#[test]
fn test_e2e_clone_from_hub_default_directory_no_workspaces_affix() {
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let original_cwd = std::env::current_dir().unwrap();
    let temp_work_dir = tempdir().unwrap();

    let blueprint_content = "repositories:\n";
    let routes = vec![
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws".to_string(),
            status_code: 200,
            body: serde_json::json!({
                "latestRevision": {
                    "version": 1,
                    "blueprintYaml": blueprint_content,
                }
            })
            .to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws/files".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
        MockRoute {
            method: "GET".to_string(),
            path_prefix: "/api/v1/projects/kyete/projecti-ws/secrets".to_string(),
            status_code: 200,
            body: serde_json::json!([]).to_string(),
        },
    ];

    let server = MockServer::start(routes);
    std::env::set_var("WS_HUB_URL", &server.url);
    std::env::set_var("WS_HUB_TOKEN", "test_token_123");

    std::env::set_current_dir(temp_work_dir.path()).unwrap();

    let cfg = ConfigLoader::load_config(None, None, true).unwrap();
    let manager = WorkspaceManager::new(cfg, None);
    let res = manager.clone_from_hub("kyete/projecti-ws", None);

    // Restore cwd immediately
    let _ = std::env::set_current_dir(&original_cwd);

    assert!(res.is_ok(), "clone_from_hub failed: {:?}", res.err());
    let cloned_path = res.unwrap();

    // Verify folder name is strictly 'projecti-ws' and has no workspaces prefix/suffix
    assert_eq!(
        cloned_path.file_name().unwrap().to_str().unwrap(),
        "projecti-ws"
    );
    assert!(!cloned_path.to_str().unwrap().contains("-workspaces"));
    assert!(!cloned_path.to_str().unwrap().contains("workspaces-"));
    assert!(cloned_path.join("repositories.yml").exists());
}

#[test]
fn test_e2e_hub_state_restore_recreates_workspace_with_named_repos() {
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let proj_dir = tempdir().unwrap();
    let bares_dir = proj_dir.path().join("bares");
    std::fs::create_dir_all(&bares_dir).unwrap();
    let bare_repo = bares_dir.join("mono.git");

    // Initialize bare git repository with initial commit and feat-ui branch
    let init_st = Command::new("git")
        .args(["init", "--bare", bare_repo.to_str().unwrap()])
        .status()
        .expect("git init failed");
    assert!(init_st.success());

    // Create a temporary clone to commit into bare
    let seed_dir = tempdir().unwrap();
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            seed_dir.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "config",
            "user.name",
            "Test",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "config",
            "user.email",
            "test@test.com",
        ])
        .status()
        .unwrap();
    std::fs::write(seed_dir.path().join("README.md"), "# mono\n").unwrap();
    Command::new("git")
        .args(["-C", seed_dir.path().to_str().unwrap(), "add", "README.md"])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "commit",
            "-m",
            "init",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "branch",
            "-M",
            "main",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "push",
            "origin",
            "main",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "checkout",
            "-b",
            "feat-ui",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-C",
            seed_dir.path().to_str().unwrap(),
            "push",
            "origin",
            "feat-ui",
        ])
        .status()
        .unwrap();

    let repo_yml_content = format!(
        "repositories:\n  mono:\n    bare: {}\n    checkout: mono\n",
        bare_repo.display()
    );
    let repo_yml = proj_dir.path().join("repositories.yml");
    std::fs::write(&repo_yml, &repo_yml_content).unwrap();

    // Mock hub route returning state where repositories contains "mono" without "name" field
    let routes = vec![MockRoute {
        method: "GET".to_string(),
        path_prefix: "/api/v1/workspaces/kyete/myproject/states/ui".to_string(),
        status_code: 200,
        body: serde_json::json!({
            "status": "success",
            "data": {
                "workspaceName": "ui",
                "stateJson": serde_json::json!({
                    "name": "ui",
                    "repositories": {
                        "mono": {
                            "branch": "feat-ui",
                            "create": true,
                            "path": "mono"
                        }
                    },
                    "wip": {}
                }).to_string()
            }
        })
        .to_string(),
    }];

    let server = MockServer::start(routes);
    std::env::set_var("WS_HUB_URL", &server.url);
    std::env::set_var("WS_HUB_TOKEN", "test_token_123");

    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(proj_dir.path()).unwrap();

    let cfg = ConfigLoader::load_config(Some(&repo_yml), None, true).unwrap();
    let manager = WorkspaceManager::new(cfg, None);

    let res = manager.hub_state_restore("ui", Some("kyete/myproject"), true);

    let _ = std::env::set_current_dir(&original_cwd);

    assert!(res.is_ok(), "hub_state_restore failed: {:?}", res.err());
    let ws_dir = proj_dir.path().join("workspaces/ui");
    assert!(ws_dir.exists(), "workspaces/ui directory must exist");
    assert!(ws_dir.join("mono").exists(), "worktree mono must exist");
}
