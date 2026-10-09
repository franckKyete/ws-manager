use std::fs;
use std::path::{Path, PathBuf};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::errors::{Result, WSError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSessionData {
    pub url: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HubClient {
    pub base_url: String,
    pub token: Option<String>,
    pub config_path: PathBuf,
    client: Client,
}

impl HubClient {
    pub fn new(url: Option<&str>, token: Option<&str>, config_path: Option<&Path>) -> Self {
        let default_config = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("ws")
            .join("hub.yml");
        let cfg_path = config_path.map(|p| p.to_path_buf()).unwrap_or(default_config);

        let (saved_url, saved_token) = Self::load_saved_config(&cfg_path);

        let final_url = url
            .map(|s| s.to_string())
            .or_else(|| std::env::var("WS_HUB_URL").ok())
            .or(saved_url)
            .unwrap_or_else(|| "https://hub.ws.dev".to_string())
            .trim_end_matches('/')
            .to_string();

        let final_token = token
            .map(|s| s.to_string())
            .or_else(|| std::env::var("WS_HUB_TOKEN").ok())
            .or(saved_token);

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            base_url: final_url,
            token: final_token,
            config_path: cfg_path,
            client,
        }
    }

    pub fn load_saved_config(path: &Path) -> (Option<String>, Option<String>) {
        if !path.is_file() {
            return (None, None);
        }
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(data) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                let url = data.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());
                let token = data.get("token").and_then(|v| v.as_str()).map(|s| s.to_string());
                return (url, token);
            }
        }
        (None, None)
    }

    pub fn save_session(&mut self, url: &str, token: &str, username: Option<&str>) -> Result<()> {
        let _ = fs::create_dir_all(self.config_path.parent().unwrap_or_else(|| Path::new(".")));
        let clean_url = url.trim_end_matches('/').to_string();

        let session = HubSessionData {
            url: clean_url.clone(),
            token: token.to_string(),
            username: username.map(|s| s.to_string()),
        };

        let yaml_str = serde_yaml::to_string(&session)?;
        fs::write(&self.config_path, yaml_str)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.config_path, fs::Permissions::from_mode(0o600));
        }

        self.base_url = clean_url;
        self.token = Some(token.to_string());
        Ok(())
    }

    pub fn clear_session(&mut self) -> bool {
        if self.config_path.is_file() {
            let _ = fs::remove_file(&self.config_path);
            self.token = None;
            true
        } else {
            false
        }
    }

    fn request(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        json_body: Option<&serde_json::Value>,
        raw_body: Option<Vec<u8>>,
        headers_extra: Option<HeaderMap>,
    ) -> Result<serde_json::Value> {
        let clean_ep = endpoint.trim_start_matches('/');
        let url = format!("{}/api/v1/{}", self.base_url, clean_ep);

        let mut req = self.client.request(method, &url);
        if let Some(ref tok) = self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {}", tok));
        }

        if let Some(hdrs) = headers_extra {
            req = req.headers(hdrs);
        }

        if let Some(body) = json_body {
            req = req.json(body);
        } else if let Some(bytes) = raw_body {
            req = req.body(bytes);
        }

        let resp = req.send().map_err(|e| WSError::Hub {
            message: format!("Network request failed: {}", e),
            status_code: 0,
            details: None,
        })?;

        let status = resp.status();
        let status_code = status.as_u16();

        let resp_bytes = resp.bytes().map_err(|e| WSError::Hub {
            message: format!("Failed reading response bytes: {}", e),
            status_code,
            details: None,
        })?;

        let parsed_json: serde_json::Value = serde_json::from_slice(&resp_bytes)
            .unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let err_msg = parsed_json.get("message")
                .or_else(|| parsed_json.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown server error")
                .to_string();

            return Err(WSError::Hub {
                message: err_msg,
                status_code,
                details: Some(parsed_json),
            });
        }

        Ok(parsed_json)
    }

    pub fn register(&self, username: &str, email: &str, password: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "username": username,
            "email": email,
            "password": password,
        });
        self.request(reqwest::Method::POST, "auth/register", Some(&body), None, None)
    }

    pub fn login(&mut self, username_or_email: &str, password: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "usernameOrEmail": username_or_email,
            "password": password,
        });
        let res = self.request(reqwest::Method::POST, "auth/login", Some(&body), None, None)?;
        if let Some(tok) = res.get("token").and_then(|t| t.as_str()) {
            let user = res.get("user").and_then(|u| u.get("username")).and_then(|u| u.as_str());
            self.save_session(&self.base_url.clone(), tok, user)?;
        }
        Ok(res)
    }

    pub fn whoami(&self) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, "auth/me", None, None, None)
    }

    pub fn create_pat(&self, name: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "name": name });
        self.request(reqwest::Method::POST, "auth/pat", Some(&body), None, None)
    }

    pub fn parse_project_identifier(identifier: &str) -> Result<(String, String)> {
        let clean = identifier.trim().trim_start_matches('@');
        let parts: Vec<&str> = clean.splitn(2, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            Ok((parts[0].to_string(), parts[1].to_string()))
        } else {
            Err(WSError::Validation(format!(
                "Invalid project identifier '{}'. Must be in 'namespace/name' format (e.g. 'org/project').",
                identifier
            )))
        }
    }

    pub fn get_project(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, &format!("projects/{}/{}", namespace, name), None, None, None)
    }

    pub fn list_projects(&self) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, "projects", None, None, None)
    }

    pub fn create_project(
        &self,
        namespace: &str,
        name: &str,
        description: Option<&str>,
        is_private: bool,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "namespace": namespace,
            "name": name,
            "description": description.unwrap_or(""),
            "isPrivate": is_private,
        });
        self.request(reqwest::Method::POST, "projects", Some(&body), None, None)
    }

    pub fn push_revision(
        &self,
        namespace: &str,
        name: &str,
        blueprint_yaml: &str,
        changelog: Option<&str>,
        version: Option<&str>,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "blueprint": blueprint_yaml,
            "changelog": changelog.unwrap_or("Updated project blueprint"),
            "version": version,
        });
        self.request(
            reqwest::Method::POST,
            &format!("projects/{}/{}/revisions", namespace, name),
            Some(&body),
            None,
            None,
        )
    }

    pub fn get_revisions(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, &format!("projects/{}/{}/revisions", namespace, name), None, None, None)
    }

    pub fn list_secrets(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, &format!("projects/{}/{}/secrets", namespace, name), None, None, None)
    }

    pub fn set_secret(
        &self,
        namespace: &str,
        name: &str,
        key: &str,
        value: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "key": key,
            "value": value,
            "repoName": repo_name,
        });
        self.request(
            reqwest::Method::POST,
            &format!("projects/{}/{}/secrets", namespace, name),
            Some(&body),
            None,
            None,
        )
    }

    pub fn set_secrets_bulk(
        &self,
        namespace: &str,
        name: &str,
        secrets: &[(String, String, Option<String>)], // (key, value, repo_name)
    ) -> Result<serde_json::Value> {
        let list: Vec<_> = secrets.iter().map(|(k, v, r)| {
            serde_json::json!({
                "key": k,
                "value": v,
                "repoName": r,
            })
        }).collect();
        let body = serde_json::json!({ "secrets": list });
        self.request(
            reqwest::Method::POST,
            &format!("projects/{}/{}/secrets/bulk", namespace, name),
            Some(&body),
            None,
            None,
        )
    }

    pub fn get_secret(
        &self,
        namespace: &str,
        name: &str,
        key: &str,
        repo_name: Option<&str>,
    ) -> Result<String> {
        let ep = if let Some(r) = repo_name {
            format!("projects/{}/{}/secrets/{}?repoName={}", namespace, name, key, r)
        } else {
            format!("projects/{}/{}/secrets/{}", namespace, name, key)
        };
        let res = self.request(reqwest::Method::GET, &ep, None, None, None)?;
        res.get("secret")
            .and_then(|s| s.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| WSError::Hub {
                message: format!("Secret '{}' not found", key),
                status_code: 404,
                details: None,
            })
    }

    pub fn delete_secret(
        &self,
        namespace: &str,
        name: &str,
        key: &str,
        repo_name: Option<&str>,
    ) -> Result<bool> {
        let ep = if let Some(r) = repo_name {
            format!("projects/{}/{}/secrets/{}?repoName={}", namespace, name, key, r)
        } else {
            format!("projects/{}/{}/secrets/{}", namespace, name, key)
        };
        let res = self.request(reqwest::Method::DELETE, &ep, None, None, None)?;
        Ok(res.get("success").and_then(|v| v.as_bool()).unwrap_or(true))
    }

    pub fn list_files(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, &format!("projects/{}/{}/files", namespace, name), None, None, None)
    }

    pub fn upload_file(
        &self,
        namespace: &str,
        name: &str,
        rel_file_path: &str,
        content_bytes: Vec<u8>,
    ) -> Result<serde_json::Value> {
        let mut headers = HeaderMap::new();
        headers.insert("X-File-Path", HeaderValue::from_str(rel_file_path).unwrap());
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));

        self.request(
            reqwest::Method::POST,
            &format!("projects/{}/{}/files", namespace, name),
            None,
            Some(content_bytes),
            Some(headers),
        )
    }

    pub fn download_file(
        &self,
        namespace: &str,
        name: &str,
        rel_file_path: &str,
    ) -> Result<Vec<u8>> {
        let clean_ep = format!("projects/{}/{}/files/download?path={}", namespace, name, rel_file_path);
        let url = format!("{}/api/v1/{}", self.base_url, clean_ep);

        let mut req = self.client.get(&url);
        if let Some(ref tok) = self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {}", tok));
        }

        let resp = req.send().map_err(|e| WSError::Hub {
            message: e.to_string(),
            status_code: 0,
            details: None,
        })?;

        if !resp.status().is_success() {
            return Err(WSError::Hub {
                message: format!("Download failed with status {}", resp.status()),
                status_code: resp.status().as_u16(),
                details: None,
            });
        }

        resp.bytes().map(|b| b.to_vec()).map_err(|e| WSError::Hub {
            message: e.to_string(),
            status_code: 0,
            details: None,
        })
    }

    pub fn save_workspace_state(
        &self,
        namespace: &str,
        name: &str,
        workspace_name: &str,
        state_data: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.request(
            reqwest::Method::POST,
            &format!("projects/{}/{}/states/{}", namespace, name, workspace_name),
            Some(state_data),
            None,
            None,
        )
    }

    pub fn get_workspace_state(
        &self,
        namespace: &str,
        name: &str,
        workspace_name: &str,
    ) -> Result<serde_json::Value> {
        self.request(
            reqwest::Method::GET,
            &format!("projects/{}/{}/states/{}", namespace, name, workspace_name),
            None,
            None,
            None,
        )
    }

    pub fn list_workspace_states(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, &format!("projects/{}/{}/states", namespace, name), None, None, None)
    }
}

impl Default for HubClient {
    fn default() -> Self {
        Self::new(None, None, None)
    }
}

