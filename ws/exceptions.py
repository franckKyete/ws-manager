"""Custom exception classes for ws manager."""


class WSException(Exception):
    """Base exception for workspace manager errors."""

    pass


class ConfigException(WSException):
    """Raised when application or repository configuration is invalid."""

    pass


class ValidationException(WSException):
    """Base class for workspace validation failures."""

    pass


class WorkspaceExistsException(ValidationException):
    """Raised when a workspace already exists on disk."""

    pass


class WorkspaceNotFoundException(ValidationException):
    """Raised when a requested workspace does not exist."""

    pass


class RepositoryNotFoundException(ValidationException):
    """Raised when a configured bare git repository does not exist or is invalid."""

    pass


class BranchNotFoundException(ValidationException):
    """Raised when an expected git branch does not exist."""

    pass


class BranchAlreadyExistsException(ValidationException):
    """Raised when attempting to create a branch that already exists."""

    pass


class GitException(WSException):
    """Raised when a git command execution fails."""

    def __init__(self, message: str, command: str | None = None, returncode: int | None = None, stderr: str | None = None):
        super().__init__(message)
        self.command = command
        self.returncode = returncode
        self.stderr = stderr


class RollbackException(WSException):
    """Raised when an error occurs during workspace creation rollback."""

    pass


class RepoFrozenException(ValidationException):
    """Raised when attempting to mutate a frozen repository in a workspace."""

    pass


class RepoAlreadyInWorkspaceException(ValidationException):
    """Raised when adding a repo that already exists in a workspace."""

    pass


class RepoNotInWorkspaceException(ValidationException):
    """Raised when removing a repo that does not exist in a workspace."""

    pass

