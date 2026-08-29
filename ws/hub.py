"""wshub Client SDK for ws workspace manager."""

import base64
import json
import logging
import os
from pathlib import Path
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
import yaml

from ws.exceptions import ConfigException

logger = logging.getLogger("ws.hub")

DEFAULT_HUB_CONFIG_PATH = Path.home() / ".config" / "ws" / "hub.yml"
DEFAULT_HUB_URL = "http://127.0.0.1:8787"


class HubException(Exception):
    """Exception raised for wshub communication errors."""

    def __init__(self, message: str, status_code: int = 500, details: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.status_code = status_code
        self.details = details or {}


class HubClient:
    """Client for interacting with the wshub cloud/server API."""

    def __init__(
        self,
        base_url: str | None = None,
        token: str | None = None,
        config_path: Path | str | None = None,
    ) -> None:
        self.config_path = Path(config_path) if config_path else DEFAULT_HUB_CONFIG_PATH
        saved_url, saved_token = self._load_saved_config()

        self.base_url = (
            base_url
            or os.environ.get("WS_HUB_URL")
            or saved_url
            or DEFAULT_HUB_URL
        ).rstrip("/")
        self.token = (
            token
            or os.environ.get("WS_HUB_TOKEN")
            or saved_token
        )

    def _load_saved_config(self) -> tuple[str | None, str | None]:
        """Load saved URL and token from ~/.config/ws/hub.yml."""
        if not self.config_path.exists() or not self.config_path.is_file():
            return None, None
        try:
            with open(self.config_path, "r", encoding="utf-8") as f:
                data = yaml.safe_load(f) or {}
                return data.get("url"), data.get("token")
        except Exception as e:
            logger.debug("Failed to read hub config from %s: %e", self.config_path, e)
            return None, None

    def save_session(self, url: str, token: str, username: str | None = None) -> None:
        """Save active hub session to ~/.config/ws/hub.yml."""
        self.config_path.parent.mkdir(parents=True, exist_ok=True)
        data = {
            "url": url.rstrip("/"),
            "token": token,
        }
        if username:
            data["username"] = username

        with open(self.config_path, "w", encoding="utf-8") as f:
            yaml.dump(data, f, default_flow_style=False)
        try:
            self.config_path.chmod(0o600)
        except Exception:
            pass

        self.base_url = data["url"]
        self.token = data["token"]

    def clear_session(self) -> bool:
        """Delete saved hub session credentials."""
        if self.config_path.exists():
            self.config_path.unlink()
            self.token = None
            return True
        return False

    def _request(
        self,
        method: str,
        endpoint: str,
        data: dict[str, Any] | None = None,
        query_params: dict[str, str] | None = None,
        requires_auth: bool = True,
    ) -> dict[str, Any]:
        """Execute an HTTP JSON request to the wshub API."""
        url = f"{self.base_url}{endpoint}"
        if query_params:
            qs = urllib.parse.urlencode({k: v for k, v in query_params.items() if v is not None})
            if qs:
                url = f"{url}?{qs}"

        headers = {
            "Accept": "application/json",
            "Content-Type": "application/json",
        }

        if requires_auth:
            if not self.token:
                raise HubException(
                    "Not authenticated with wshub. Please run 'ws hub login' first.",
                    status_code=401,
                )
            headers["Authorization"] = f"Bearer {self.token}"

        if isinstance(data, dict):
            data = {k: v for k, v in data.items() if v is not None}
        body_bytes = json.dumps(data).encode("utf-8") if data is not None else None
        req = urllib.request.Request(url, data=body_bytes, headers=headers, method=method)

        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                resp_bytes = response.read()
                if not resp_bytes:
                    return {}
                return json.loads(resp_bytes.decode("utf-8"))
        except urllib.error.HTTPError as e:
            err_body = e.read().decode("utf-8", errors="replace")
            try:
                err_json = json.loads(err_body)
                # Check for Zod error issues: { error: { issues: [ { path, message } ] } } or { message }
                if "error" in err_json and isinstance(err_json["error"], dict) and "issues" in err_json["error"]:
                    issue_msgs = []
                    for issue in err_json["error"]["issues"]:
                        path_str = ".".join(str(p) for p in issue.get("path", []))
                        msg_str = issue.get("message", "validation error")
                        issue_msgs.append(f"{path_str}: {msg_str}" if path_str else msg_str)
                    msg = f"Validation Error ({', '.join(issue_msgs)})"
                else:
                    msg = err_json.get("message", f"HTTP {e.code}: {e.reason}")
                details = err_json.get("details", {})
            except Exception:
                msg = f"HTTP {e.code}: {e.reason} ({err_body})"
                details = {}
            raise HubException(msg, status_code=e.code, details=details) from e
        except urllib.error.URLError as e:
            raise HubException(
                f"Failed to connect to wshub at '{self.base_url}': {e.reason}",
                status_code=503,
            ) from e

    # -------------------------------------------------------------------------
    # Authentication & User Management
    # -------------------------------------------------------------------------

    def register(self, username: str, email: str, password: str) -> dict[str, Any]:
        """Register a new user account."""
        res = self._request(
            "POST",
            "/api/v1/auth/register",
            {"username": username, "email": email, "password": password},
            requires_auth=False,
        )
        return res.get("data", {})

    def login(self, username_or_email: str, password: str) -> dict[str, Any]:
        """Authenticate with wshub using credentials."""
        res = self._request(
            "POST",
            "/api/v1/auth/login",
            {"usernameOrEmail": username_or_email, "password": password},
            requires_auth=False,
        )
        data = res.get("data", {})
        token = data.get("token")
        user = data.get("user", {})
        if token:
            self.save_session(self.base_url, token, user.get("username"))
        return data

    def whoami(self) -> dict[str, Any]:
        """Get details for currently authenticated user."""
        res = self._request("GET", "/api/v1/auth/whoami", requires_auth=True)
        return res.get("data", {}).get("user", {})

    def create_pat(self, name: str) -> dict[str, Any]:
        """Create a new Personal Access Token."""
        res = self._request("POST", "/api/v1/auth/tokens", {"name": name}, requires_auth=True)
        return res.get("data", {})

    # -------------------------------------------------------------------------
    # Project Blueprints & Revisions
    # -------------------------------------------------------------------------

    def parse_project_identifier(self, identifier: str) -> tuple[str, str]:
        """Parse 'org/project' or 'project' into (namespace, name)."""
        clean = identifier.strip().rstrip("/")
        if clean.startswith("wshub:"):
            clean = clean[6:]
        if "/" in clean:
            org, name = clean.split("/", 1)
            return org.lower(), name.lower()

        # If no namespace provided, try whoami username, else 'default'
        try:
            user = self.whoami()
            username = user.get("username", "personal")
        except Exception:
            username = "personal"
        return username.lower(), clean.lower()

    def get_project(self, namespace: str, name: str) -> dict[str, Any]:
        """Retrieve project summary and latest revision."""
        res = self._request("GET", f"/api/v1/projects/{namespace}/{name}", requires_auth=True)
        return res.get("data", {})

    def list_projects(self) -> list[dict[str, Any]]:
        """List all accessible projects."""
        res = self._request("GET", "/api/v1/projects", requires_auth=True)
        return res.get("data", [])

    def create_project(
        self,
        namespace: str,
        name: str,
        blueprint_yaml: str,
        description: str | None = None,
        scripts_json: str | None = None,
        changelog: str | None = None,
    ) -> dict[str, Any]:
        """Register a new project on wshub."""
        payload = {
            "namespace": namespace,
            "name": name,
            "blueprintYaml": blueprint_yaml,
            "description": description,
            "scriptsJson": scripts_json,
            "changelog": changelog,
        }
        res = self._request("POST", "/api/v1/projects", payload, requires_auth=True)
        return res.get("data", {})

    def push_revision(
        self,
        namespace: str,
        name: str,
        blueprint_yaml: str,
        scripts_json: str | None = None,
        changelog: str = "Update configuration",
    ) -> dict[str, Any]:
        """Push an updated blueprint revision to an existing project."""
        payload = {
            "blueprintYaml": blueprint_yaml,
            "scriptsJson": scripts_json,
            "changelog": changelog,
        }
        res = self._request("POST", f"/api/v1/projects/{namespace}/{name}/revisions", payload, requires_auth=True)
        return res.get("data", {})

    def get_revisions(self, namespace: str, name: str) -> list[dict[str, Any]]:
        """Get revision history for a project."""
        res = self._request("GET", f"/api/v1/projects/{namespace}/{name}/revisions", requires_auth=True)
        return res.get("data", [])

    # -------------------------------------------------------------------------
    # Zero-Git Secrets Vault
    # -------------------------------------------------------------------------

    def list_secrets(self, namespace: str, name: str) -> list[dict[str, Any]]:
        """List decrypted secrets for an authorized project."""
        res = self._request("GET", f"/api/v1/projects/{namespace}/{name}/secrets", requires_auth=True)
        return res.get("data", [])

    def set_secret(
        self,
        namespace: str,
        name: str,
        key: str,
        value: str,
        repo_name: str | None = None,
    ) -> None:
        """Set or update a single secret in the project vault."""
        payload = {"key": key, "value": value, "repoName": repo_name}
        self._request("PUT", f"/api/v1/projects/{namespace}/{name}/secrets", payload, requires_auth=True)

    def set_secrets_bulk(
        self,
        namespace: str,
        name: str,
        secrets: dict[str, str],
        repo_name: str | None = None,
    ) -> None:
        """Bulk update multiple secrets."""
        payload = {"secrets": secrets, "repoName": repo_name}
        self._request("POST", f"/api/v1/projects/{namespace}/{name}/secrets/bulk", payload, requires_auth=True)

    def get_secret(self, namespace: str, name: str, key: str, repo_name: str | None = None) -> str:
        """Retrieve a specific secret value."""
        params = {"repo": repo_name} if repo_name else None
        res = self._request("GET", f"/api/v1/projects/{namespace}/{name}/secrets/{key}", query_params=params, requires_auth=True)
        return res.get("data", {}).get("value", "")

    def delete_secret(self, namespace: str, name: str, key: str, repo_name: str | None = None) -> bool:
        """Delete a secret from the vault."""
        params = {"repo": repo_name} if repo_name else None
        res = self._request("DELETE", f"/api/v1/projects/{namespace}/{name}/secrets/{key}", query_params=params, requires_auth=True)
        return res.get("data", {}).get("deleted", False)

    # -------------------------------------------------------------------------
    # Sensitive Files Store
    # -------------------------------------------------------------------------

    def list_files(self, namespace: str, name: str) -> list[dict[str, Any]]:
        """List sensitive files stored in the project vault."""
        res = self._request("GET", f"/api/v1/projects/{namespace}/{name}/files", requires_auth=True)
        return res.get("data", [])

    def upload_file(self, namespace: str, name: str, rel_file_path: str, content_bytes: bytes) -> dict[str, Any]:
        """Upload and encrypt a sensitive file to the hub vault."""
        b64 = base64.b64encode(content_bytes).decode("utf-8")
        payload = {"filePath": rel_file_path, "contentBase64": b64}
        res = self._request("POST", f"/api/v1/projects/{namespace}/{name}/files", payload, requires_auth=True)
        return res.get("data", {})

    def download_file(self, namespace: str, name: str, rel_file_path: str) -> bytes:
        """Download and decrypt a sensitive file from the hub vault."""
        res = self._request(
            "GET",
            f"/api/v1/projects/{namespace}/{name}/files/download",
            query_params={"path": rel_file_path},
            requires_auth=True,
        )
        b64 = res.get("data", {}).get("contentBase64", "")
        return base64.b64decode(b64.encode("utf-8"))

    # -------------------------------------------------------------------------
    # Workspace State Synchronization (Cross-Machine Resumption)
    # -------------------------------------------------------------------------

    def save_workspace_state(
        self,
        namespace: str,
        name: str,
        workspace_name: str,
        state_dict: dict[str, Any],
    ) -> dict[str, Any]:
        """Save active workspace branch checkouts, locks, and local config to hub."""
        payload = {
            "workspaceName": workspace_name,
            "stateJson": json.dumps(state_dict),
        }
        res = self._request("POST", f"/api/v1/workspaces/{namespace}/{name}/states", payload, requires_auth=True)
        return res.get("data", {})

    def get_workspace_state(self, namespace: str, name: str, workspace_name: str) -> dict[str, Any]:
        """Retrieve saved workspace state for cross-machine resumption."""
        res = self._request(
            "GET",
            f"/api/v1/workspaces/{namespace}/{name}/states/{workspace_name.lstrip('@')}",
            requires_auth=True,
        )
        data = res.get("data", {})
        raw_json = data.get("stateJson", "{}")
        try:
            return json.loads(raw_json)
        except Exception:
            return {}

    def list_workspace_states(self, namespace: str, name: str) -> list[dict[str, Any]]:
        """List all saved workspace states for a project."""
        res = self._request("GET", f"/api/v1/workspaces/{namespace}/{name}/states", requires_auth=True)
        return res.get("data", [])
