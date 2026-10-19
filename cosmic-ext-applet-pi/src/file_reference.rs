use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pi_rpc::protocol::ImageContent;

/// Extracts an active `@` query if the input is currently ending with `@<query>`.
/// Returns `Some((before_token, query))` where `before_token` is the text preceding `@<query>`.
pub fn extract_at_query(input: &str) -> Option<(&str, &str)> {
    // If the input ends with whitespace, the user is not actively typing an `@` query.
    if input.ends_with(char::is_whitespace) {
        return None;
    }

    // Find the start of the last word separated by whitespace
    let last_word_start = input.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
    let last_word = &input[last_word_start..];

    if let Some(query) = last_word.strip_prefix('@') {
        // Only trigger if query doesn't contain a quote or newline
        if !query.contains('"') && !query.contains('\'') && !query.contains('\n') {
            return Some((&input[..last_word_start], query));
        }
    }
    None
}

/// Applies completion of a file path into the input string.
/// Replaces the active `@query` with `@{completed_path} `.
pub fn apply_at_completion(input: &str, completed_path: &str) -> String {
    let before = match extract_at_query(input) {
        Some((b, _)) => b,
        None => input,
    };
    format!("{before}@{completed_path} ")
}

/// Finds files matching `query` within `root` directory.
/// Returns relative paths from `root`, up to `limit` items.
pub fn find_matching_files(root: &Path, query: &str, limit: usize) -> Vec<String> {
    if !root.is_dir() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();

    // Standard directories to ignore to keep searches fast and relevant
    let ignore_dirs = [
        ".git",
        "target",
        "node_modules",
        ".cache",
        ".cargo",
        "build",
        "dist",
        ".idea",
        ".zed",
        ".vscode",
    ];

    let walker = walkdir::WalkDir::new(root)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let file_name = entry.file_name().to_string_lossy();
            if entry.file_type().is_dir() {
                if ignore_dirs.contains(&file_name.as_ref()) {
                    return false;
                }
                if file_name.starts_with('.') && !query_lower.starts_with('.') {
                    return false;
                }
            }
            true
        });

    for entry in walker.flatten() {
        if entry.file_type().is_dir() {
            continue;
        }

        let rel_path = match entry.path().strip_prefix(root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if query_lower.is_empty() {
            matches.push(rel_path);
        } else {
            let rel_lower = rel_path.to_lowercase();
            let file_name_lower = entry.file_name().to_string_lossy().to_lowercase();

            if file_name_lower.contains(&query_lower) || rel_lower.contains(&query_lower) {
                matches.push(rel_path);
            }
        }

        if matches.len() >= limit * 3 {
            break;
        }
    }

    // Sort by match relevance:
    // 1. Exact filename match
    // 2. Filename starts with query
    // 3. Path starts with query
    // 4. Shorter depth & length
    matches.sort_by(|a, b| {
        let a_name = a.split('/').next_back().unwrap_or(a).to_lowercase();
        let b_name = b.split('/').next_back().unwrap_or(b).to_lowercase();
        let a_exact = a_name == query_lower;
        let b_exact = b_name == query_lower;
        if a_exact != b_exact {
            return b_exact.cmp(&a_exact);
        }
        let a_starts = a_name.starts_with(&query_lower);
        let b_starts = b_name.starts_with(&query_lower);
        if a_starts != b_starts {
            return b_starts.cmp(&a_starts);
        }
        let a_depth = a.matches('/').count();
        let b_depth = b.matches('/').count();
        if a_depth != b_depth {
            return a_depth.cmp(&b_depth);
        }
        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
    });

    matches.truncate(limit);
    matches
}

/// Scans prompt for `@path` references and:
/// - Inlines existing text files (<= 128KB) with `<file name="path">...</file>` tags.
/// - Loads and encodes existing image files as `ImageContent`.
///
/// Returns the processed prompt message and any extracted images.
pub fn preprocess_prompt(text: &str, working_dir: &Path) -> (String, Vec<ImageContent>) {
    let mut images = Vec::new();
    let mut file_attachments_text = String::new();
    let mut seen_paths = HashSet::new();

    for word in text.split_whitespace() {
        if let Some(path_str) = word.strip_prefix('@') {
            let clean_path = path_str
                .trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ':' || c == ';');
            if clean_path.is_empty() {
                continue;
            }

            let full_path = if Path::new(clean_path).is_absolute() {
                PathBuf::from(clean_path)
            } else {
                working_dir.join(clean_path)
            };

            if !full_path.is_file() {
                continue;
            }

            if !seen_paths.insert(full_path.clone()) {
                continue;
            }

            let ext = full_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let mime_opt = match ext.as_str() {
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "webp" => Some("image/webp"),
                "gif" => Some("image/gif"),
                "bmp" => Some("image/bmp"),
                "svg" => Some("image/svg+xml"),
                _ => None,
            };

            if let Some(mime) = mime_opt {
                if let Ok(bytes) = std::fs::read(&full_path) {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    images.push(ImageContent::new(mime, b64));
                }
            } else if let Ok(metadata) = std::fs::metadata(&full_path) {
                // Text files up to 128KB
                if metadata.len() > 0
                    && metadata.len() <= 128 * 1024
                    && let Ok(content) = std::fs::read_to_string(&full_path)
                {
                    file_attachments_text.push_str(&format!(
                        "\n\n<file name=\"{clean_path}\">\n{content}\n</file>"
                    ));
                }
            }
        }
    }

    let final_message = if file_attachments_text.is_empty() {
        text.to_string()
    } else {
        format!("{text}{file_attachments_text}")
    };

    (final_message, images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_extract_at_query() {
        assert_eq!(extract_at_query("@"), Some(("", "")));
        assert_eq!(extract_at_query("@main"), Some(("", "main")));
        assert_eq!(
            extract_at_query("Please review @Cargo"),
            Some(("Please review ", "Cargo"))
        );
        assert_eq!(
            extract_at_query("Check @src/lib.rs and @ma"),
            Some(("Check @src/lib.rs and ", "ma"))
        );
        assert_eq!(extract_at_query("Completed @main.rs "), None);
        assert_eq!(extract_at_query(""), None);
        assert_eq!(extract_at_query("/help"), None);
        assert_eq!(extract_at_query("no at symbol"), None);
    }

    #[test]
    fn test_apply_at_completion() {
        assert_eq!(apply_at_completion("@", "src/main.rs"), "@src/main.rs ");
        assert_eq!(
            apply_at_completion("Please review @Car", "Cargo.toml"),
            "Please review @Cargo.toml "
        );
        assert_eq!(
            apply_at_completion("Check @src/lib.rs and @ma", "src/main.rs"),
            "Check @src/lib.rs and @src/main.rs "
        );
    }

    #[test]
    fn test_find_matching_files() {
        let temp_dir = std::env::temp_dir().join(format!("pi_test_matching_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("src")).unwrap();

        File::create(temp_dir.join("Cargo.toml"))
            .unwrap()
            .write_all(b"[package]")
            .unwrap();
        File::create(temp_dir.join("src/main.rs"))
            .unwrap()
            .write_all(b"fn main() {}")
            .unwrap();
        File::create(temp_dir.join("src/lib.rs"))
            .unwrap()
            .write_all(b"pub fn foo() {}")
            .unwrap();

        let matches = find_matching_files(&temp_dir, "main", 5);
        assert_eq!(matches, vec!["src/main.rs"]);

        let all_matches = find_matching_files(&temp_dir, "", 10);
        assert!(all_matches.contains(&"Cargo.toml".to_string()));
        assert!(all_matches.contains(&"src/main.rs".to_string()));
        assert!(all_matches.contains(&"src/lib.rs".to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_preprocess_prompt_text_and_dedup() {
        let temp_dir = std::env::temp_dir().join(format!("pi_test_preprocess_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let note_path = temp_dir.join("note.txt");
        File::create(&note_path)
            .unwrap()
            .write_all(b"Hello from note")
            .unwrap();

        let prompt = "Check @note.txt and @note.txt please";
        let (processed, images) = preprocess_prompt(prompt, &temp_dir);

        assert!(images.is_empty());
        assert!(processed.contains("<file name=\"note.txt\">"));
        assert!(processed.contains("Hello from note"));
        // Deduplication: should only contain the file block once
        assert_eq!(processed.matches("<file name=\"note.txt\">").count(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
