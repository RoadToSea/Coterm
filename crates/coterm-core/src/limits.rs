//! Shared capacity policy for operation retention and text-file interfaces.
//! Schemas and execution must consume these values rather than repeat numeric limits.

/// Maximum serialized JSON bytes retained as one operation result.
pub const OPERATION_RESULT_MAX_BYTES: usize = 192 * 1024;
/// Maximum UTF-8 bytes in one file read or write, before JSON encoding.
pub const FILE_CONTENT_MAX_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 bytes in a remote file path; controls are rejected separately.
pub const FILE_PATH_MAX_BYTES: usize = 4096;
/// Fallback execution deadline in milliseconds when the caller omits timeoutMs.
/// Explicit positive, representable caller deadlines take precedence.
pub const DEFAULT_FILE_TIMEOUT_MS: u64 = 15_000;
