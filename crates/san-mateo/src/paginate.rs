//! Cursor, link and offset pagination, from the provider `proxy.paginate`
//! block.
//!
//! [`Paginator`] gives the query for each page. It does no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How the offset moves to the next page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OffsetMethod {
    /// Add the number of records on the page.
    #[default]
    #[serde(rename = "by-response-size")]
    ByResponseSize,
    /// Add one.
    #[serde(rename = "per-page")]
    PerPage,
}

/// A pagination config. The `type` field selects the kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PaginationConfig {
    /// The reply holds a cursor for the next page.
    Cursor {
        /// Dotted path to the next cursor in the reply body.
        cursor_path_in_response: String,
        /// Query parameter that takes the cursor.
        cursor_name_in_request: String,
        /// Query parameter that takes the page size.
        #[serde(default)]
        limit_name_in_request: Option<String>,
        /// Dotted path to the records in the reply body.
        #[serde(default)]
        response_path: Option<String>,
    },
    /// The reply holds a link to the next page.
    Link {
        /// The `rel` of the `Link` header. Default `next`.
        #[serde(default)]
        link_rel_in_response_header: Option<String>,
        /// Dotted path to the next link in the reply body.
        #[serde(default)]
        link_path_in_response_body: Option<String>,
        /// Query parameter that takes the page size.
        #[serde(default)]
        limit_name_in_request: Option<String>,
        /// Dotted path to the records in the reply body.
        #[serde(default)]
        response_path: Option<String>,
    },
    /// The request holds an offset.
    Offset {
        /// Query parameter that takes the offset.
        offset_name_in_request: String,
        /// The first offset. Default 0.
        #[serde(default)]
        offset_start_value: Option<u64>,
        /// How the offset moves.
        #[serde(default)]
        offset_calculation_method: Option<OffsetMethod>,
        /// Query parameter that takes the page size.
        #[serde(default)]
        limit_name_in_request: Option<String>,
        /// Dotted path to the records in the reply body.
        #[serde(default)]
        response_path: Option<String>,
    },
}

impl PaginationConfig {
    /// Returns the dotted path to the records.
    #[must_use]
    pub fn response_path(&self) -> Option<&str> {
        match self {
            Self::Cursor { response_path, .. }
            | Self::Link { response_path, .. }
            | Self::Offset { response_path, .. } => response_path.as_deref(),
        }
    }

    fn limit_name(&self) -> Option<&str> {
        match self {
            Self::Cursor { limit_name_in_request, .. }
            | Self::Link { limit_name_in_request, .. }
            | Self::Offset { limit_name_in_request, .. } => limit_name_in_request.as_deref(),
        }
    }
}

/// The query for one page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageQuery {
    /// Query parameters to set.
    pub params: Vec<(String, String)>,
    /// A link to fetch in place of the original endpoint (link pagination).
    /// It can be absolute or a path.
    pub link: Option<String>,
}

/// Gives the query for each page.
#[derive(Debug, Clone)]
pub struct Paginator {
    config: PaginationConfig,
    limit: Option<u64>,
    offset: u64,
    last_link: Option<String>,
}

impl Paginator {
    /// Makes a paginator. `limit` is the page size to ask for.
    #[must_use]
    pub fn new(config: PaginationConfig, limit: Option<u64>) -> Self {
        let offset = match &config {
            PaginationConfig::Offset { offset_start_value, .. } => offset_start_value.unwrap_or(0),
            _ => 0,
        };
        Self {
            config,
            limit,
            offset,
            last_link: None,
        }
    }

    fn limit_param(&self) -> Option<(String, String)> {
        Some((self.config.limit_name()?.to_owned(), self.limit?.to_string()))
    }

    /// Returns the query for the first page.
    #[must_use]
    pub fn first(&self) -> PageQuery {
        let mut params = Vec::new();
        if let PaginationConfig::Offset { offset_name_in_request, .. } = &self.config {
            params.push((offset_name_in_request.clone(), self.offset.to_string()));
        }
        params.extend(self.limit_param());
        PageQuery { params, link: None }
    }

    /// Reads a page reply. Returns the query for the next page, or `None` at
    /// the end.
    pub fn advance(&mut self, headers: &[(String, String)], body: &Value) -> Option<PageQuery> {
        let count = self.records(body).len();
        match &self.config {
            PaginationConfig::Cursor { cursor_path_in_response, cursor_name_in_request, .. } => {
                if count == 0 {
                    return None;
                }
                let next = match crate::template::lookup(body, cursor_path_in_response)? {
                    Value::String(s) if !s.is_empty() => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => return None,
                };
                let mut params = vec![(cursor_name_in_request.clone(), next)];
                params.extend(self.limit_param());
                Some(PageQuery { params, link: None })
            }
            PaginationConfig::Link { link_rel_in_response_header, link_path_in_response_body, .. } => {
                let rel = link_rel_in_response_header.as_deref().unwrap_or("next");
                let from_body = link_path_in_response_body
                    .as_deref()
                    .and_then(|p| crate::template::lookup(body, p))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let next = from_body.or_else(|| {
                    crate::transport::header_value(headers, "link").and_then(|h| parse_link_header(h, rel))
                })?;
                if self.last_link.as_deref() == Some(next.as_str()) {
                    return None;
                }
                self.last_link = Some(next.clone());
                Some(PageQuery { params: Vec::new(), link: Some(next) })
            }
            PaginationConfig::Offset { offset_name_in_request, offset_calculation_method, .. } => {
                if count == 0 {
                    return None;
                }
                if let Some(limit) = self.limit {
                    if (count as u64) < limit {
                        return None;
                    }
                }
                let step = match offset_calculation_method.unwrap_or_default() {
                    OffsetMethod::ByResponseSize => count as u64,
                    OffsetMethod::PerPage => 1,
                };
                self.offset = self.offset.saturating_add(step);
                let mut params = vec![(offset_name_in_request.clone(), self.offset.to_string())];
                params.extend(self.limit_param());
                Some(PageQuery { params, link: None })
            }
        }
    }

    /// Returns the records of a page reply.
    #[must_use]
    pub fn records(&self, body: &Value) -> Vec<Value> {
        records_from(body, self.config.response_path())
    }
}

/// Returns the records at `path`, or the body itself when `path` is `None`.
/// A value that is not an array gives no records.
#[must_use]
pub fn records_from(body: &Value, path: Option<&str>) -> Vec<Value> {
    let target = match path {
        Some(p) => crate::template::lookup(body, p),
        None => Some(body),
    };
    match target {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Returns the URL with `rel` from an RFC 8288 `Link` header value.
#[must_use]
pub fn parse_link_header(value: &str, rel: &str) -> Option<String> {
    for link in value.split(',') {
        let mut parts = link.split(';');
        let Some(target) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(url) = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')) else {
            continue;
        };
        let has_rel = parts.any(|p| {
            let p = p.trim();
            p.strip_prefix("rel")
                .map(str::trim_start)
                .and_then(|p| p.strip_prefix('='))
                .is_some_and(|v| v.trim().trim_matches('"').split_whitespace().any(|r| r.eq_ignore_ascii_case(rel)))
        });
        if has_rel {
            return Some(url.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cursor() -> PaginationConfig {
        serde_json::from_value(json!({
            "type": "cursor",
            "cursor_path_in_response": "paging.next.after",
            "cursor_name_in_request": "after",
            "limit_name_in_request": "limit",
            "response_path": "results"
        }))
        .unwrap()
    }

    #[test]
    fn parses_each_kind() {
        assert!(matches!(cursor(), PaginationConfig::Cursor { .. }));
        let l: PaginationConfig = serde_json::from_value(json!({"type": "link", "link_rel_in_response_header": "next"})).unwrap();
        assert!(matches!(l, PaginationConfig::Link { .. }));
        let o: PaginationConfig = serde_json::from_value(json!({"type": "offset", "offset_name_in_request": "offset", "offset_calculation_method": "per-page"})).unwrap();
        assert!(matches!(o, PaginationConfig::Offset { offset_calculation_method: Some(OffsetMethod::PerPage), .. }));
        assert!(serde_json::from_value::<PaginationConfig>(json!({"type": "page"})).is_err());
    }

    #[test]
    fn cursor_flow() {
        let mut p = Paginator::new(cursor(), Some(50));
        assert_eq!(p.first().params, vec![("limit".to_owned(), "50".to_owned())]);
        let page1 = json!({"results": [{"id": 1}], "paging": {"next": {"after": "c2"}}});
        assert_eq!(p.records(&page1).len(), 1);
        let next = p.advance(&[], &page1).unwrap();
        assert_eq!(next.params, vec![("after".to_owned(), "c2".to_owned()), ("limit".to_owned(), "50".to_owned())]);
        assert_eq!(p.advance(&[], &json!({"results": [{"id": 2}]})), None);
        assert_eq!(p.advance(&[], &json!({"results": [], "paging": {"next": {"after": "c3"}}})), None);
        assert_eq!(p.advance(&[], &json!({"results": [{"id": 3}], "paging": {"next": {"after": ""}}})), None);
    }

    #[test]
    fn link_header_flow() {
        let cfg: PaginationConfig = serde_json::from_value(json!({"type": "link", "limit_name_in_request": "per_page"})).unwrap();
        let mut p = Paginator::new(cfg, Some(10));
        assert_eq!(p.first().params, vec![("per_page".to_owned(), "10".to_owned())]);
        let h = vec![("link".to_owned(), "<https://api.example.com/x?page=2>; rel=\"next\", <https://api.example.com/x?page=9>; rel=\"last\"".to_owned())];
        let next = p.advance(&h, &json!([{"id": 1}])).unwrap();
        assert_eq!(next.link.as_deref(), Some("https://api.example.com/x?page=2"));
        assert!(next.params.is_empty());
        // The same link again ends the loop.
        assert_eq!(p.advance(&h, &json!([{"id": 1}])), None);
        assert_eq!(p.advance(&[], &json!([{"id": 1}])), None);
    }

    #[test]
    fn link_in_body() {
        let cfg: PaginationConfig = serde_json::from_value(json!({"type": "link", "link_path_in_response_body": "links.next", "response_path": "data"})).unwrap();
        let mut p = Paginator::new(cfg, None);
        let next = p.advance(&[], &json!({"data": [1], "links": {"next": "/v1/items?page=2"}})).unwrap();
        assert_eq!(next.link.as_deref(), Some("/v1/items?page=2"));
        assert_eq!(p.advance(&[], &json!({"data": [1], "links": {"next": null}})), None);
    }

    #[test]
    fn offset_flow() {
        let cfg: PaginationConfig = serde_json::from_value(json!({"type": "offset", "offset_name_in_request": "offset", "limit_name_in_request": "limit"})).unwrap();
        let mut p = Paginator::new(cfg, Some(2));
        assert_eq!(p.first().params, vec![("offset".to_owned(), "0".to_owned()), ("limit".to_owned(), "2".to_owned())]);
        let next = p.advance(&[], &json!([1, 2])).unwrap();
        assert_eq!(next.params[0], ("offset".to_owned(), "2".to_owned()));
        // Fewer records than the limit: the end.
        assert_eq!(p.advance(&[], &json!([3])), None);
    }

    #[test]
    fn offset_per_page_and_start() {
        let cfg: PaginationConfig = serde_json::from_value(json!({"type": "offset", "offset_name_in_request": "page", "offset_start_value": 1, "offset_calculation_method": "per-page"})).unwrap();
        let mut p = Paginator::new(cfg, None);
        assert_eq!(p.first().params, vec![("page".to_owned(), "1".to_owned())]);
        assert_eq!(p.advance(&[], &json!([1, 2, 3])).unwrap().params, vec![("page".to_owned(), "2".to_owned())]);
        assert_eq!(p.advance(&[], &json!([])), None);
    }

    #[test]
    fn records_paths() {
        assert_eq!(records_from(&json!([1, 2]), None).len(), 2);
        assert_eq!(records_from(&json!({"a": {"b": [1]}}), Some("a.b")).len(), 1);
        assert!(records_from(&json!({"a": 1}), Some("a")).is_empty());
        assert!(records_from(&json!({"a": 1}), None).is_empty());
    }

    #[test]
    fn link_header_parsing() {
        let v = r#"<https://a/x?p=1>; rel="prev",   <https://a/x?p=3> ; rel=next"#;
        assert_eq!(parse_link_header(v, "next").as_deref(), Some("https://a/x?p=3"));
        assert_eq!(parse_link_header(v, "prev").as_deref(), Some("https://a/x?p=1"));
        assert_eq!(parse_link_header(v, "last"), None);
        assert_eq!(parse_link_header(r#"<https://a/1>; rel="next last""#, "last").as_deref(), Some("https://a/1"));
        assert_eq!(parse_link_header("garbage", "next"), None);
    }
}
