//! 简化版 PEP 440 版本解析与比较（移植自 Python 启动器 repo.py）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub raw: String,
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// "", "dev", "a", "b", "rc", "post"
    pub prerelease: &'static str,
    pub prerelease_num: u64,
}

fn prerelease_order(p: &str) -> u8 {
    match p {
        "dev" => 0,
        "a" => 1,
        "b" => 2,
        "rc" => 3,
        "" => 4,
        "post" => 5,
        _ => 4,
    }
}

fn normalize_pre_label(label: &str) -> &'static str {
    match label {
        "alpha" | "a" => "a",
        "beta" | "b" => "b",
        "rc" | "c" | "pre" | "preview" => "rc",
        "dev" => "dev",
        "post" | "rev" | "r" => "post",
        _ => "",
    }
}

impl Version {
    pub fn new(raw: &str) -> Version {
        let raw = raw.trim();
        let mut v = Version {
            raw: raw.to_string(),
            major: 0,
            minor: 0,
            patch: 0,
            prerelease: "",
            prerelease_num: 0,
        };
        let s = raw.to_ascii_lowercase();
        let s = s.strip_prefix('v').unwrap_or(&s);

        // 拆掉 local 段 (+xxx) 与数字/预发布部分
        let core: String = s
            .split('+')
            .next()
            .unwrap_or(s)
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
            .collect();
        let core = core.as_str();

        // 数字段
        let mut num_part = String::new();
        let mut rest = "";
        for (i, c) in core.char_indices() {
            if c.is_ascii_digit() || c == '.' {
                num_part.push(c);
            } else {
                rest = &core[i..];
                break;
            }
        }
        let nums: Vec<u64> = num_part
            .split('.')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect();
        if !nums.is_empty() {
            v.major = nums[0];
            v.minor = nums.get(1).copied().unwrap_or(0);
            v.patch = nums.get(2).copied().unwrap_or(0);
        }

        // 预发布段：dev / a / b / rc / post
        let rest_lower = rest.trim_start_matches(['-', '_', '.']);
        let (label, num_str) = split_pre(rest_lower);
        if let Some(label) = label {
            v.prerelease = normalize_pre_label(label);
            v.prerelease_num = num_str.unwrap_or(0);
        }
        v
    }

}

fn split_pre(rest: &str) -> (Option<&str>, Option<u64>) {
    let label_candidates = [
        ("dev", 3usize),
        ("preview", 7),
        ("alpha", 5),
        ("beta", 4),
        ("post", 4),
        ("rc", 2),
        ("pre", 3),
        ("rev", 3),
        ("a", 1),
        ("b", 1),
        ("c", 1),
        ("r", 1),
    ];
    for (label, len) in label_candidates {
        if rest.len() >= len && rest[..len].eq_ignore_ascii_case(label) {
            let num_str = rest[len..].trim_start_matches(['-', '_', '.']);
            let num: String = num_str.chars().take_while(|c| c.is_ascii_digit()).collect();
            let num = if num.is_empty() { None } else { num.parse().ok() };
            return (Some(label), num);
        }
    }
    // 纯数字后缀（如 "1.0-1" 的 post 形式）在 rest 为空时无法判断，忽略
    (None, None)
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            o => return o,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            o => return o,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            o => return o,
        }
        match prerelease_order(self.prerelease).cmp(&prerelease_order(other.prerelease)) {
            Ordering::Equal => {}
            o => return o,
        }
        if self.prerelease == other.prerelease {
            self.prerelease_num.cmp(&other.prerelease_num)
        } else {
            Ordering::Equal
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare() {
        assert!(Version::new("2025.9b2") < Version::new("2025.9"));
        assert!(Version::new("2025.9") < Version::new("2025.10"));
        assert!(Version::new("v2026.10.1") > Version::new("2026.10"));
        assert!(Version::new("1.0.dev1") < Version::new("1.0a1"));
        assert!(Version::new("1.0.post1") > Version::new("1.0"));
        assert_eq!(Version::new("1.0b2"), Version::new("1.0b2"));
    }
}
