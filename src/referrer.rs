//! Lecture des URL et classement du canal d'acquisition d'une visite.

pub const CHANNEL_DIRECT: i16 = 0;
pub const CHANNEL_SEARCH: i16 = 1;
pub const CHANNEL_SOCIAL: i16 = 2;
pub const CHANNEL_REFERRAL: i16 = 3;
pub const CHANNEL_CAMPAIGN: i16 = 4;
pub const CHANNEL_AI: i16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    /// Nom d'hôte en minuscules, sans port.
    pub host: String,
    /// Chemin, toujours avec la barre initiale, sans paramètres ni ancre.
    pub path: String,
    pub query: String,
}

/// Découpe une URL http(s). Renvoie None pour tout autre schéma ou une URL sans hôte.
pub fn parse_url(raw: &str) -> Option<ParsedUrl> {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .or_else(|| raw.get(..8).filter(|p| p.eq_ignore_ascii_case("https://")).map(|_| &raw[8..]))
        .or_else(|| raw.get(..7).filter(|p| p.eq_ignore_ascii_case("http://")).map(|_| &raw[7..]))?;
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let authority = authority.rsplit('@').next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (path, query) = match tail.find('?') {
        Some(i) => (&tail[..i], &tail[i + 1..]),
        None => (tail, ""),
    };
    let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
    Some(ParsedUrl { host, path, query: query.to_string() })
}

/// Valeur décodée d'un paramètre d'URL, ou None s'il est absent ou vide.
pub fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key && !v.is_empty()).then(|| percent_decode(v))
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let decoded = bytes
                    .get(i + 1..i + 3)
                    .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match decoded {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Acquisition {
    pub channel: i16,
    pub referrer_host: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
}

const SEARCH: &[&str] = &[
    "google.",
    "bing.com",
    "duckduckgo.com",
    "qwant.com",
    "ecosia.org",
    "yahoo.",
    "yandex.",
    "baidu.com",
    "startpage.com",
    "search.brave.com",
    "lilo.org",
    "kagi.com",
];
const SOCIAL: &[&str] = &[
    "facebook.com",
    "instagram.com",
    "twitter.com",
    "x.com",
    "t.co",
    "linkedin.com",
    "lnkd.in",
    "youtube.com",
    "youtu.be",
    "tiktok.com",
    "pinterest.",
    "reddit.com",
    "threads.net",
    "bsky.app",
    "snapchat.com",
    "whatsapp.com",
    "t.me",
    "telegram.org",
];
const AI: &[&str] = &[
    "chatgpt.com",
    "chat.openai.com",
    "perplexity.ai",
    "claude.ai",
    "gemini.google.com",
    "copilot.microsoft.com",
    "chat.mistral.ai",
    "you.com",
    "phind.com",
];

fn host_matches(host: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix('.') {
            // « google. » : google.fr, www.google.com… mais pas notgoogle.fr.
            host.split('.').any(|label| label == prefix)
        } else {
            host == *p || host.ends_with(&format!(".{p}"))
        }
    })
}

fn truncate(value: String, max: usize) -> String {
    if value.chars().count() <= max { value } else { value.chars().take(max).collect() }
}

/// Classe l'origine d'une visite d'après l'URL d'arrivée et le référent.
pub fn classify(page: &ParsedUrl, referrer: Option<&str>) -> Acquisition {
    let utm = |key: &str| query_param(&page.query, key).map(|v| truncate(v, 200));
    let mut acq = Acquisition {
        utm_source: utm("utm_source"),
        utm_medium: utm("utm_medium"),
        utm_campaign: utm("utm_campaign"),
        ..Default::default()
    };
    let paid_click = ["gclid", "fbclid", "msclkid"].iter().any(|k| query_param(&page.query, k).is_some());

    let referrer = referrer.and_then(parse_url).map(|u| u.host.trim_start_matches("www.").to_string());
    let own = page.host.trim_start_matches("www.");
    let external = referrer.filter(|host| host != own && !host.is_empty());

    acq.channel = if acq.utm_source.is_some() || acq.utm_medium.is_some() || acq.utm_campaign.is_some() || paid_click {
        CHANNEL_CAMPAIGN
    } else {
        match external.as_deref() {
            None => CHANNEL_DIRECT,
            // Les assistants avant la recherche : gemini.google.com contient « google ».
            Some(h) if host_matches(h, AI) => CHANNEL_AI,
            Some(h) if host_matches(h, SEARCH) => CHANNEL_SEARCH,
            Some(h) if host_matches(h, SOCIAL) => CHANNEL_SOCIAL,
            Some(_) => CHANNEL_REFERRAL,
        }
    };
    acq.referrer_host = external.map(|h| truncate(h, 200));
    acq
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> ParsedUrl {
        parse_url(s).unwrap()
    }

    #[test]
    fn parses_urls() {
        assert_eq!(
            url("https://Demo.Test:8443/a/b?x=1#h"),
            ParsedUrl { host: "demo.test".into(), path: "/a/b".into(), query: "x=1".into() }
        );
        assert_eq!(url("http://demo.test").path, "/");
        assert_eq!(url("https://demo.test?x=1").query, "x=1");
        assert_eq!(url("https://user:pw@demo.test/p").host, "demo.test");
        assert_eq!(url("HTTPS://demo.test/").host, "demo.test");
        assert!(parse_url("javascript:alert(1)").is_none());
        assert!(parse_url("file:///etc/passwd").is_none());
        assert!(parse_url("https://").is_none());
        assert!(parse_url("").is_none());
    }

    #[test]
    fn decodes_params() {
        assert_eq!(query_param("a=1&utm_source=lettre%20d%27info&b=", "utm_source").as_deref(), Some("lettre d'info"));
        assert_eq!(query_param("q=a+b", "q").as_deref(), Some("a b"));
        assert_eq!(query_param("q=100%", "q").as_deref(), Some("100%"));
        assert_eq!(query_param("q=%zz%4", "q").as_deref(), Some("%zz%4"));
        assert_eq!(query_param("b=", "b"), None);
        assert_eq!(query_param("", "b"), None);
    }

    #[test]
    fn classifies_channels() {
        let page = url("https://www.demo.test/produits");
        assert_eq!(classify(&page, None).channel, CHANNEL_DIRECT);
        assert_eq!(classify(&page, Some("https://demo.test/")).channel, CHANNEL_DIRECT, "navigation interne");
        assert_eq!(classify(&page, Some("https://demo.test/")).referrer_host, None);
        assert_eq!(classify(&page, Some("https://www.google.fr/search?q=x")).channel, CHANNEL_SEARCH);
        assert_eq!(classify(&page, Some("https://www.google.fr/")).referrer_host.as_deref(), Some("google.fr"));
        assert_eq!(classify(&page, Some("https://notgoogle.fr/")).channel, CHANNEL_REFERRAL);
        assert_eq!(classify(&page, Some("https://l.facebook.com/l.php")).channel, CHANNEL_SOCIAL);
        assert_eq!(classify(&page, Some("https://t.co/abc")).channel, CHANNEL_SOCIAL);
        assert_eq!(classify(&page, Some("https://gemini.google.com/app")).channel, CHANNEL_AI);
        assert_eq!(classify(&page, Some("https://chatgpt.com/")).channel, CHANNEL_AI);
        assert_eq!(classify(&page, Some("https://exemple-partenaire.fr/article")).channel, CHANNEL_REFERRAL);
        assert_eq!(classify(&page, Some("android-app://com.google.android.gm")).channel, CHANNEL_DIRECT);

        let campaign = classify(
            &url("https://demo.test/?utm_source=lettre&utm_medium=email&utm_campaign=no%C3%ABl"),
            Some("https://www.google.fr/"),
        );
        assert_eq!(campaign.channel, CHANNEL_CAMPAIGN);
        assert_eq!(campaign.utm_campaign.as_deref(), Some("noël"));
        assert_eq!(classify(&url("https://demo.test/?gclid=abc"), None).channel, CHANNEL_CAMPAIGN);
    }
}
