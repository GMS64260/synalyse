//! Lecture minimale de l'en-tête User-Agent : famille de navigateur, système, type d'appareil, robots.
//! Volontairement grossier : aucune version, aucun modèle d'appareil n'est conservé.

pub const DEVICE_UNKNOWN: i16 = 0;
pub const DEVICE_DESKTOP: i16 = 1;
pub const DEVICE_MOBILE: i16 = 2;
pub const DEVICE_TABLET: i16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agent {
    pub device: i16,
    pub browser: &'static str,
    pub os: &'static str,
    pub bot: bool,
}

const BOT_MARKERS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "slurp",
    "headless",
    "lighthouse",
    "pingdom",
    "uptime",
    "monitor",
    "curl/",
    "wget/",
    "python-",
    "go-http-client",
    "okhttp",
    "axios",
    "node-fetch",
    "scrapy",
    "phantomjs",
    "facebookexternalhit",
    "whatsapp",
    "preview",
    "feedfetcher",
    "httpclient",
    "java/",
    "libwww",
    "selenium",
    "playwright",
    "puppeteer",
];

pub fn parse(ua: &str) -> Agent {
    let lower = ua.to_ascii_lowercase();
    let bot = ua.trim().is_empty() || BOT_MARKERS.iter().any(|m| lower.contains(m));

    let browser = if lower.contains("edg/") || lower.contains("edga/") || lower.contains("edgios/") {
        "Edge"
    } else if lower.contains("opr/") || lower.contains("opera") {
        "Opera"
    } else if lower.contains("samsungbrowser") {
        "Samsung Internet"
    } else if lower.contains("firefox/") || lower.contains("fxios/") {
        "Firefox"
    } else if lower.contains("chrome/") || lower.contains("crios/") || lower.contains("chromium/") {
        "Chrome"
    } else if lower.contains("safari/") && lower.contains("version/") {
        "Safari"
    } else {
        "Autre"
    };

    let os = if lower.contains("windows") {
        "Windows"
    } else if lower.contains("android") {
        "Android"
    } else if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
        "iOS"
    } else if lower.contains("mac os x") || lower.contains("macintosh") {
        "macOS"
    } else if lower.contains("cros") {
        "ChromeOS"
    } else if lower.contains("linux") {
        "Linux"
    } else {
        "Autre"
    };

    let device = if lower.contains("ipad")
        || lower.contains("tablet")
        || (lower.contains("android") && !lower.contains("mobile"))
    {
        DEVICE_TABLET
    } else if lower.contains("mobi") || lower.contains("iphone") || lower.contains("ipod") || lower.contains("android")
    {
        DEVICE_MOBILE
    } else if os == "Autre" {
        DEVICE_UNKNOWN
    } else {
        DEVICE_DESKTOP
    };

    Agent { device, browser, os, bot }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_agents() {
        let chrome = parse(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        );
        assert_eq!(
            (chrome.browser, chrome.os, chrome.device, chrome.bot),
            ("Chrome", "Windows", DEVICE_DESKTOP, false)
        );
        let iphone = parse(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
        );
        assert_eq!((iphone.browser, iphone.os, iphone.device), ("Safari", "iOS", DEVICE_MOBILE));
        let firefox = parse("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:131.0) Gecko/20100101 Firefox/131.0");
        assert_eq!((firefox.browser, firefox.os, firefox.device), ("Firefox", "macOS", DEVICE_DESKTOP));
        let pixel = parse(
            "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
        );
        assert_eq!((pixel.browser, pixel.os, pixel.device), ("Chrome", "Android", DEVICE_MOBILE));
        let tab = parse(
            "Mozilla/5.0 (Linux; Android 14; SM-X710) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        );
        assert_eq!(tab.device, DEVICE_TABLET);
        let edge = parse(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
        );
        assert_eq!(edge.browser, "Edge");
    }

    #[test]
    fn bots() {
        for ua in [
            "",
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "curl/8.5.0",
            "python-requests/2.32",
            "Mozilla/5.0 HeadlessChrome/130.0",
        ] {
            assert!(parse(ua).bot, "{ua}");
        }
    }
}
