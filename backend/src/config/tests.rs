use std::collections::HashMap;

use secrecy::ExposeSecret;

use super::*;

const GOOD_SECRET: &str = "9be2320de5bf8e5b07297443f66bcb7ac546fee5bf2c3f91d7c070e385a43d99";

fn base() -> HashMap<&'static str, String> {
    HashMap::from([
        ("R16A_ENV", "production".into()),
        ("R16A_PUBLIC_URL", "https://stream.example.test".into()),
        (
            "R16A_DATABASE_URL",
            "postgres://r16a_stream_app:Xq7vR2mK9pL4tW8z@db:5432/r16a_stream".into(),
        ),
        (
            "R16A_S3_INTERNAL_ENDPOINTS",
            "http://10.0.0.1:3900, http://10.0.0.2:3900".into(),
        ),
        ("R16A_S3_PUBLIC_ENDPOINT", "https://stream.example.test".into()),
        ("R16A_S3_ACCESS_KEY_ID", "GK0123456789abcdef01234567".into()),
        ("R16A_S3_SECRET_ACCESS_KEY", GOOD_SECRET.into()),
        (
            "R16A_OIDC_ISSUER",
            "https://auth.example.test/application/o/r16a-stream/".into(),
        ),
        ("R16A_OIDC_WEB_CLIENT_ID", "r16a-stream".into()),
        ("R16A_OIDC_WEB_CLIENT_SECRET", GOOD_SECRET.into()),
        (
            "R16A_OIDC_MOBILE_ISSUER",
            "https://auth.example.test/application/o/r16a-stream-mobile/".into(),
        ),
        ("R16A_OIDC_MOBILE_CLIENT_ID", "r16a-stream-mobile".into()),
        ("R16A_SESSION_KEY", GOOD_KEY.into()),
        ("R16A_TRUSTED_PROXY_CIDRS", "10.42.0.0/16".into()),
        (
            "R16A_TMDB_TOKEN",
            "eyJhbGciOiJIUzI1NiJ9.tmdb-read-token-4f8c2a91d7e3b605".into(),
        ),
    ])
}

/// 32 bytes, standard base64.
const GOOD_KEY: &str = "q1ZK2vN0c8Rr3mXy7TfB5wLhJd9UeP4sGa6Ck2Yo1iM=";

fn load(vars: &HashMap<&'static str, String>) -> Result<ApiConfig, ConfigError> {
    ApiConfig::from_lookup(&|n| vars.get(n).cloned())
}

#[test]
fn valid_production_config_loads() {
    let cfg = load(&base()).expect("valid config");
    assert_eq!(cfg.env, Env::Production);
    assert_eq!(cfg.log_format, LogFormat::Json);
    assert_eq!(cfg.s3.internal_endpoints.len(), 2);
    assert_eq!(cfg.s3.secret_access_key.expose_secret(), GOOD_SECRET);
}

#[test]
fn unset_env_means_production() {
    let mut v = base();
    v.remove("R16A_ENV");
    assert_eq!(load(&v).expect("config").env, Env::Production);
}

#[test]
fn missing_secret_is_refused_by_name() {
    let mut v = base();
    v.remove("R16A_S3_SECRET_ACCESS_KEY");
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_S3_SECRET_ACCESS_KEY"
        }
    );
}

#[test]
fn empty_secret_counts_as_missing() {
    let mut v = base();
    v.insert("R16A_S3_SECRET_ACCESS_KEY", "   ".into());
    assert!(matches!(load(&v), Err(ConfigError::Missing { .. })));
}

#[test]
fn placeholders_are_refused() {
    for placeholder in [
        "CHANGE_ME",
        "changeme-please-0123456789abcdef",
        "REPLACE_WITH_A_REAL_SECRET_VALUE_1234",
    ] {
        let mut v = base();
        v.insert("R16A_S3_SECRET_ACCESS_KEY", placeholder.into());
        let err = load(&v).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::Invalid {
                    name: "R16A_S3_SECRET_ACCESS_KEY",
                    ..
                }
            ),
            "{placeholder}"
        );
    }
}

#[test]
fn placeholder_database_password_is_refused() {
    let mut v = base();
    v.insert(
        "R16A_DATABASE_URL",
        "postgres://r16a_stream_app:CHANGE_ME@db:5432/r16a_stream".into(),
    );
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_DATABASE_URL",
            ..
        })
    ));
}

#[test]
fn repetitive_secret_is_refused() {
    let mut v = base();
    v.insert("R16A_S3_SECRET_ACCESS_KEY", "a".repeat(64));
    assert!(matches!(load(&v), Err(ConfigError::Invalid { .. })));
}

#[test]
fn dev_values_are_refused_in_production_and_accepted_in_development() {
    let mut v = base();
    v.insert(
        "R16A_DATABASE_URL",
        "postgres://r16a_stream_app:dev-only-app-password@db:5432/r16a_stream".into(),
    );
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_DATABASE_URL",
            ..
        })
    ));

    v.insert("R16A_ENV", "development".into());
    v.insert("R16A_PUBLIC_URL", "http://localhost:4200".into());
    assert!(load(&v).is_ok());
}

#[test]
fn published_dev_garage_key_is_refused_in_production() {
    let mut v = base();
    v.insert("R16A_S3_ACCESS_KEY_ID", DEV_GARAGE_KEY_IDS[0].into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_S3_ACCESS_KEY_ID",
            ..
        })
    ));
}

#[test]
fn malformed_garage_key_id_is_refused() {
    let mut v = base();
    v.insert("R16A_S3_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_S3_ACCESS_KEY_ID",
            ..
        })
    ));
}

#[test]
fn insecure_cookie_with_https_is_refused() {
    let mut v = base();
    v.insert("R16A_ENV", "development".into());
    v.insert("R16A_DEV_INSECURE_COOKIE", "true".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_DEV_INSECURE_COOKIE",
            ..
        })
    ));
}

#[test]
fn production_requires_https_origin() {
    let mut v = base();
    v.insert("R16A_PUBLIC_URL", "http://stream.example.test".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_PUBLIC_URL",
            ..
        })
    ));
}

#[test]
fn zero_timeout_is_refused() {
    let mut v = base();
    v.insert("R16A_S3_TIMEOUT_MS", "0".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_S3_TIMEOUT_MS",
            ..
        })
    ));
}

#[test]
fn errors_never_contain_the_value() {
    let mut v = base();
    v.insert("R16A_S3_SECRET_ACCESS_KEY", "changeme-SECRETVALUE-0123456789".into());
    let text = load(&v).unwrap_err().to_string();
    assert!(!text.contains("SECRETVALUE"), "{text}");
}

#[test]
fn debug_output_redacts_secrets() {
    let cfg = load(&base()).expect("valid config");
    let text = format!("{cfg:?}");
    assert!(!text.contains(GOOD_SECRET));
    assert!(!text.contains("Xq7vR2mK9pL4tW8z"));
    assert!(!text.contains("7, 8, 9"), "session key bytes stay hidden");
}

#[test]
fn oidc_issuers_must_be_https_in_production() {
    let mut v = base();
    v.insert(
        "R16A_OIDC_ISSUER",
        "http://auth.example.test/application/o/r16a-stream/".into(),
    );
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Invalid {
            name: "R16A_OIDC_ISSUER",
            reason: "must be https in production".into()
        }
    );
}

#[test]
fn session_key_must_be_32_random_bytes() {
    for (value, why) in [
        ("c2hvcnQ=", "too short"),
        ("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", "repetitive"),
        ("CHANGE_ME_CHANGE_ME_CHANGE_ME_CHANGE_ME_CHANGE", "placeholder"),
        ("BwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkq", "40 bytes"),
    ] {
        let mut v = base();
        v.insert("R16A_SESSION_KEY", value.into());
        assert!(
            matches!(
                load(&v),
                Err(ConfigError::Invalid {
                    name: "R16A_SESSION_KEY",
                    ..
                } | ConfigError::Missing {
                    name: "R16A_SESSION_KEY"
                })
            ),
            "{why}"
        );
    }
    let mut v = base();
    v.remove("R16A_SESSION_KEY");
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_SESSION_KEY"
        }
    );
}

#[test]
fn web_client_secret_has_no_default_and_no_placeholder() {
    let mut v = base();
    v.remove("R16A_OIDC_WEB_CLIENT_SECRET");
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_OIDC_WEB_CLIENT_SECRET"
        }
    );
    v.insert(
        "R16A_OIDC_WEB_CLIENT_SECRET",
        "changeme-changeme-changeme-changeme".into(),
    );
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_OIDC_WEB_CLIENT_SECRET",
            ..
        })
    ));
}

#[test]
fn trusted_proxy_cidrs_are_parsed_or_refused() {
    let mut v = base();
    v.insert("R16A_TRUSTED_PROXY_CIDRS", "10.42.0.0/16, fd00::/8".into());
    assert_eq!(load(&v).expect("config").auth.trusted_proxies.len(), 2);
    v.insert("R16A_TRUSTED_PROXY_CIDRS", "10.42.0.0/40".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_TRUSTED_PROXY_CIDRS",
            ..
        })
    ));
}

#[test]
fn published_dev_session_key_is_refused_in_production() {
    let mut v = base();
    v.insert("R16A_SESSION_KEY", DEV_SESSION_KEYS[0].into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_SESSION_KEY",
            ..
        })
    ));
}

#[test]
fn production_requires_the_trusted_proxy_range() {
    let mut v = base();
    v.remove("R16A_TRUSTED_PROXY_CIDRS");
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_TRUSTED_PROXY_CIDRS"
        }
    );
}

#[test]
fn tmdb_token_is_required_and_language_checked() {
    let mut v = base();
    v.remove("R16A_TMDB_TOKEN");
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_TMDB_TOKEN"
        }
    );
    let mut v = base();
    v.insert("R16A_TMDB_LANGUAGE", "english".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_TMDB_LANGUAGE",
            ..
        })
    ));
    let mut v = base();
    v.insert("R16A_TMDB_BASE_URL", "http://api.themoviedb.org/3/".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_TMDB_BASE_URL",
            ..
        })
    ));
    let cfg = load(&base()).expect("defaults");
    assert_eq!(cfg.tmdb.base_url.as_str(), "https://api.themoviedb.org/3/");
    assert_eq!(cfg.tmdb.language, "en-US");
}

#[test]
fn gc_grace_must_outlast_the_longest_playback_token() {
    let mut v = base();
    v.insert("R16A_PLAYBACK_MAX_LIFETIME_HOURS", "24".into());
    assert!(matches!(
        load(&v),
        Err(ConfigError::Invalid {
            name: "R16A_GC_GRACE_HOURS",
            ..
        })
    ));
    v.insert("R16A_GC_GRACE_HOURS", "25".into());
    assert_eq!(load(&v).expect("config").playback_max_lifetime_hours, 24);
}

#[test]
fn metrics_token_is_required_only_with_endpoints() {
    let mut v = base();
    assert!(load(&v).expect("config").storage.metrics_token.is_none());
    v.insert("R16A_GARAGE_METRICS_ENDPOINTS", "http://10.0.0.1:3903/metrics".into());
    assert_eq!(
        load(&v).unwrap_err(),
        ConfigError::Missing {
            name: "R16A_GARAGE_METRICS_TOKEN"
        }
    );
    v.insert("R16A_GARAGE_METRICS_TOKEN", GOOD_SECRET.into());
    assert_eq!(load(&v).expect("config").storage.metrics_endpoints.len(), 1);
}

#[test]
fn retention_defaults_to_delete() {
    let mut v = base();
    assert_eq!(load(&v).expect("config").storage.retention, Retention::Delete);
    v.insert("R16A_SOURCE_RETENTION", "keep".into());
    assert_eq!(load(&v).expect("config").storage.retention, Retention::Keep);
    v.insert("R16A_SOURCE_RETENTION", "maybe".into());
    assert!(load(&v).is_err());
}
