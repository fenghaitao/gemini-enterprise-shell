//! SharePoint folder scoping for the Search API.
//!
//! With a federated SharePoint datastore, a KQL `path:"<folder URL>"` restriction in the
//! search query is evaluated by SharePoint itself (recursively, with the caller's own
//! permissions). This module turns a folder URL as copied from a browser into that
//! restriction, and checks result URLs against the folder so callers can fail closed.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use reqwest::Url;

use crate::error::{Error, Result};

/// Characters escaped when rebuilding the folder URL. SharePoint result URLs use `%20`
/// for spaces, and a literal space inside `path:"…"` matches nothing.
const PATH_ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// A normalized SharePoint folder, e.g.
/// `https://example.sharepoint.com/sites/eng/Shared%20Documents/Specs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePointFolder {
    url: String,
    host: String,
    /// Decoded, lower-cased path without a trailing slash, for containment checks.
    folder_key: String,
}

impl SharePointFolder {
    /// Accepts a folder URL from the address bar, with or without a trailing slash,
    /// `Forms/AllItems.aspx` or `?id=/sites/…` viewer URLs, and literal or `%20` spaces.
    /// Sharing links (`/:f:/…`) are rejected because they cannot be resolved locally.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.contains('"') {
            return Err(Error::validation(
                "SharePoint folder URL must not contain double quotes",
            ));
        }
        let url = Url::parse(input).map_err(|_| {
            Error::validation("invalid SharePoint folder URL; copy it from the browser address bar")
        })?;
        if !is_safe_sharepoint_url(&url) {
            return Err(Error::validation(
                "SharePoint folder URL must be https and contain no credentials or custom port",
            ));
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| Error::validation("SharePoint folder URL needs a host"))?
            .to_ascii_lowercase();
        if url.path().starts_with("/:") {
            return Err(Error::validation(
                "SharePoint sharing links (…/:f:/…) cannot be resolved locally; open the folder and copy the address bar URL instead",
            ));
        }
        // Library views put the folder in `?id=/sites/…` (already decoded by query_pairs).
        let mut path = match url.query_pairs().find(|(key, _)| key == "id") {
            Some((_, id)) => id.into_owned(),
            None => percent_decode_str(url.path())
                .decode_utf8()
                .map_err(|_| Error::validation("SharePoint folder URL path is not valid UTF-8"))?
                .into_owned(),
        };
        for suffix in ["/Forms/AllItems.aspx", "/AllItems.aspx"] {
            if path.len() >= suffix.len()
                && path[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
            {
                path.truncate(path.len() - suffix.len());
            }
        }
        let path = path.trim_end_matches('/');
        if !path.starts_with('/') || path.len() < 2 || path.contains('"') || has_dot_segment(path) {
            return Err(Error::validation(
                "SharePoint folder URL must point at a site, library or folder path",
            ));
        }
        let encoded = utf8_percent_encode(path, PATH_ESCAPE).to_string();
        Ok(Self {
            url: format!("https://{host}{encoded}"),
            host,
            folder_key: path.to_lowercase(),
        })
    }

    /// The normalized folder URL (spaces as `%20`, no trailing slash).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The KQL restriction to append to a search query. Never a negated `-path:`.
    pub fn kql(&self) -> String {
        format!("path:\"{}\"", self.url)
    }

    /// Whether `uri` is a file or folder strictly inside this folder (recursive).
    /// Ignores query/fragment (`?web=1`), percent-encoding and path case; the host must match.
    pub fn contains(&self, uri: &str) -> bool {
        let Ok(url) = Url::parse(uri.trim()) else {
            return false;
        };
        if !is_safe_sharepoint_url(&url) {
            return false;
        }
        if !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(&self.host))
        {
            return false;
        }
        let Ok(path) = percent_decode_str(url.path()).decode_utf8() else {
            return false;
        };
        // `Url::parse` already resolves `..`/`%2e%2e`; reject any that survive decoding anyway.
        if has_dot_segment(&path) {
            return false;
        }
        path.to_lowercase()
            .strip_prefix(&self.folder_key)
            .is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
    }
}

/// Comparable form of a result URL: lower-cased host and decoded, lower-cased path,
/// without query or fragment (`?web=1`). `None` if it is not a usable URL.
pub(crate) fn normalized_uri(uri: &str) -> Option<String> {
    let url = Url::parse(uri.trim()).ok()?;
    if !is_safe_sharepoint_url(&url) {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let path = percent_decode_str(url.path()).decode_utf8().ok()?;
    if has_dot_segment(&path) {
        return None;
    }
    Some(format!(
        "{host}{}",
        path.trim_end_matches('/').to_lowercase()
    ))
}

fn is_safe_sharepoint_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn has_dot_segment(path: &str) -> bool {
    path.split('/')
        .any(|segment| segment == "." || segment == "..")
}

/// Search keywords to combine with a folder restriction. Rejects anything that could
/// change the restriction: `path:` operators and unbalanced quotes or parentheses.
pub(crate) fn validate_search_terms(terms: &str) -> Result<&str> {
    let terms = terms.trim();
    if terms.is_empty() || terms.len() > 1024 || terms.chars().any(char::is_control) {
        return Err(Error::validation(
            "search terms must be 1-1024 printable characters",
        ));
    }
    if terms.to_ascii_lowercase().contains("path:") {
        return Err(Error::validation(
            "search terms must not contain path:; use the SharePoint folder option instead",
        ));
    }
    if !terms.matches('"').count().is_multiple_of(2) {
        return Err(Error::validation(
            "search terms have an unbalanced double quote",
        ));
    }
    let mut depth = 0i32;
    for c in terms.chars() {
        depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            break;
        }
    }
    if depth != 0 {
        return Err(Error::validation(
            "search terms have unbalanced parentheses",
        ));
    }
    Ok(terms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs";

    #[test]
    fn folder_url_forms_normalize_to_one_kql_restriction() {
        for input in [
            FOLDER,
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/",
            "https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs",
            "  https://EXAMPLE.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs  ",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Forms/AllItems.aspx?id=%2Fsites%2Feng%2FShared%20Documents%2FTeam%2FSpecs&viewid=abc",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/Forms/AllItems.aspx",
        ] {
            let folder = SharePointFolder::parse(input).unwrap();
            assert_eq!(folder.url(), FOLDER, "{input}");
            assert_eq!(folder.kql(), format!("path:\"{FOLDER}\""));
            assert!(!folder.kql().starts_with('-'));
        }
    }

    #[test]
    fn bad_folder_urls_are_rejected() {
        for input in [
            "http://example.sharepoint.com/sites/eng",
            "https://user:pw@example.sharepoint.com/sites/eng",
            "https://example.sharepoint.com:8443/sites/eng",
            "https://example.sharepoint.com/:f:/s/eng/EabcDEF",
            "https://example.sharepoint.com/",
            "not a url",
            "https://example.sharepoint.com/sites/eng\"",
        ] {
            assert!(SharePointFolder::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn containment_is_strict_recursive_and_ignores_query_case_and_encoding() {
        let folder = SharePointFolder::parse(FOLDER).unwrap();
        for inside in [
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/a.txt?web=1",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/sub/b.txt?",
            "https://example.sharepoint.com/sites/eng/Shared Documents/team/specs/c.pdf",
            "https://EXAMPLE.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/d.docx#x",
        ] {
            assert!(folder.contains(inside), "{inside}");
        }
        // Encoded dot segments must never escape the folder.
        for escape in [
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/%2e%2e/secret.txt",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/%2E%2E/x.txt",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/.%2e/x.txt",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/../x.txt",
        ] {
            assert!(!folder.contains(escape), "{escape}");
        }
        assert!(has_dot_segment("/sites/eng/Team/Specs/../x"));
        assert!(!has_dot_segment("/sites/eng/Team/Specs/..x/a.b"));
        for outside in [
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs-old/a.txt",
            "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/a.txt",
            "https://other.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/a.txt",
            "http://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/a.txt",
            "https://user@example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs/a.txt",
            "https://example.sharepoint.com:8443/sites/eng/Shared%20Documents/Team/Specs/a.txt",
            "not a url",
        ] {
            assert!(!folder.contains(outside), "{outside}");
        }
    }

    #[test]
    fn search_terms_cannot_change_the_folder_restriction() {
        for ok in [
            "HWSync req/ack",
            "\"exact phrase\" other",
            "(alpha OR beta) gamma",
        ] {
            assert!(validate_search_terms(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "   ",
            "x path:\"https://example.sharepoint.com/sites/other\"",
            "PATH:foo",
            "unbalanced \" quote",
            "(open",
            "close)",
            ") reopen (",
        ] {
            assert!(validate_search_terms(bad).is_err(), "{bad:?}");
        }
    }
}
