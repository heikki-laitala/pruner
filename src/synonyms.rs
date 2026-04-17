//! Static synonym clusters for programming concepts.
//!
//! When a query keyword appears in a cluster, the other cluster members are
//! added as synonym keywords. Synonyms participate in scoring at reduced weight
//! (see [`SYNONYM_IDF_FACTOR`]) so they help recall without drowning out exact
//! matches.
//!
//! Keep clusters tight: members should be near-synonyms ("login" ≈ "signin"),
//! not loose neighbours ("login" vs "session"). Loose links inflate keyword
//! counts and pull in unrelated files.

use std::collections::HashSet;

/// Weight multiplier applied to synonym IDFs. Half-strength means a synonym
/// hit never ties a real keyword hit, but still contributes when the file or
/// symbol only matches through the synonym.
pub const SYNONYM_IDF_FACTOR: f64 = 0.5;

/// Programming-concept synonym groups. All members must be lowercase.
pub const SYNONYM_CLUSTERS: &[&[&str]] = &[
    // Auth — user-facing "login" and implementation "authenticate" are the
    // same flow in practice; merged so a prompt about one finds the other.
    &[
        "auth",
        "authenticate",
        "authentication",
        "login",
        "signin",
        "logon",
    ],
    &["logout", "signout", "logoff"],
    &["authorize", "authorization"],
    &["credential", "credentials"],
    &["password", "passwd"],
    &["token", "jwt", "bearer"],
    // Networking
    &["endpoint", "route", "handler"],
    &["request", "req"],
    &["response", "resp", "reply"],
    &["websocket", "ws", "socket"],
    &["connect", "connection"],
    &["disconnect", "teardown", "close"],
    &["reconnect", "reconnection"],
    &["retry", "retries"],
    &["timeout", "deadline"],
    // Rate limiting / backpressure
    &["ratelimit", "throttle", "quota"],
    &["backpressure", "backoff"],
    // Caching
    &["cache", "caching", "memoize"],
    &["invalidate", "evict", "expire"],
    // Database
    &["database", "db"],
    &["query", "queries"],
    &["migration", "migrate"],
    &["transaction", "txn"],
    &["schema", "ddl"],
    // Messaging / async
    &["publish", "publisher", "produce", "producer"],
    &["subscribe", "subscriber", "consume", "consumer"],
    &["queue", "topic", "channel"],
    &["async", "asynchronous"],
    &["concurrent", "parallel"],
    // Logging / observability
    &["log", "logger", "logging"],
    &["trace", "tracing"],
    &["metric", "metrics", "telemetry"],
    &["exception", "panic"],
    // Testing
    &["test", "spec"],
    &["mock", "stub", "fake"],
    &["assert", "expect"],
    // Config
    &["config", "configuration", "settings"],
    &["env", "environment"],
    &["flag", "toggle"],
    &["secret", "credential"],
    // Build / deploy
    &["build", "compile"],
    &["deploy", "release", "rollout"],
    &["container", "docker"],
    // IO / parsing
    &["parse", "parser", "parsing"],
    &["serialize", "marshal", "encode"],
    &["deserialize", "unmarshal", "decode"],
    // Errors
    &["error", "err"],
    &["failure", "fail"],
    // Data
    &["list", "array"],
    &["map", "dict", "dictionary"],
    // Lifecycle
    &["init", "initialize", "initialise", "setup", "bootstrap"],
    &["start", "startup"],
    &["stop", "shutdown"],
    // UI
    &["render", "draw"],
    &["component", "widget"],
    &["event", "signal"],
];

/// Expand `keywords` with programming-concept synonyms.
///
/// Returns `(expanded, synonym_set)`:
/// - `expanded`: original keywords followed by any synonyms that weren't
///   already present. Order preserved; duplicates removed.
/// - `synonym_set`: the keywords that were *added* (i.e., every entry in
///   `expanded` that didn't appear in the input). Callers use this set to
///   apply [`SYNONYM_IDF_FACTOR`] so synonyms score below originals.
pub fn expand_with_synonyms(keywords: &[String]) -> (Vec<String>, HashSet<String>) {
    let mut seen: HashSet<String> = keywords.iter().map(|k| k.to_lowercase()).collect();
    let mut expanded: Vec<String> = keywords.to_vec();
    let mut synonyms: HashSet<String> = HashSet::new();

    for kw in keywords {
        let kw_lower = kw.to_lowercase();
        for cluster in SYNONYM_CLUSTERS {
            if !cluster.iter().any(|&s| s == kw_lower) {
                continue;
            }
            for &term in *cluster {
                if seen.insert(term.to_string()) {
                    expanded.push(term.to_string());
                    synonyms.insert(term.to_string());
                }
            }
        }
    }

    (expanded, synonyms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_expands_to_signin() {
        let (exp, syns) = expand_with_synonyms(&["login".into()]);
        assert!(exp.contains(&"signin".into()));
        assert!(syns.contains("signin"));
        assert!(
            !syns.contains("login"),
            "original should not be in synonym set"
        );
    }

    #[test]
    fn auth_expands_to_authenticate() {
        let (exp, _) = expand_with_synonyms(&["auth".into()]);
        assert!(exp.contains(&"authenticate".into()));
        assert!(exp.contains(&"authentication".into()));
    }

    #[test]
    fn unknown_keyword_does_not_expand() {
        let (exp, syns) = expand_with_synonyms(&["foobar".into()]);
        assert_eq!(exp, vec!["foobar".to_string()]);
        assert!(syns.is_empty());
    }

    #[test]
    fn original_keyword_not_duplicated() {
        let (exp, syns) = expand_with_synonyms(&["login".into(), "signin".into()]);
        let signin_count = exp.iter().filter(|k| *k == "signin").count();
        assert_eq!(signin_count, 1, "signin must not appear twice");
        assert!(
            !syns.contains("signin"),
            "signin was in input — not a synonym"
        );
        assert!(
            !syns.contains("login"),
            "login was in input — not a synonym"
        );
    }

    #[test]
    fn case_insensitive_match() {
        let (exp, _) = expand_with_synonyms(&["Login".into()]);
        assert!(exp.contains(&"signin".into()));
    }

    #[test]
    fn original_keywords_come_first() {
        let (exp, _) = expand_with_synonyms(&["login".into(), "foo".into()]);
        assert_eq!(exp[0], "login");
        assert_eq!(exp[1], "foo");
    }

    #[test]
    fn clusters_are_lowercase_and_non_empty() {
        for cluster in SYNONYM_CLUSTERS {
            assert!(
                cluster.len() >= 2,
                "cluster must have ≥2 members: {cluster:?}"
            );
            for term in *cluster {
                assert_eq!(
                    *term,
                    term.to_lowercase(),
                    "cluster member must be lowercase: {term}"
                );
                assert!(!term.is_empty());
            }
        }
    }
}
