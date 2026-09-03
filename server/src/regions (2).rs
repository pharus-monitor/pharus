/// ISO 3166-1 alpha-2 → display name for the regions servers are commonly hosted in.
/// Unknown codes fall back to the code itself so grouping still works.
const REGIONS: &[(&str, &str)] = &[
    ("CN", "中国大陆"),
    ("HK", "中国香港"),
    ("TW", "中国台湾"),
    ("MO", "中国澳门"),
    ("JP", "日本"),
    ("KR", "韩国"),
    ("SG", "新加坡"),
    ("MY", "马来西亚"),
    ("TH", "泰国"),
    ("VN", "越南"),
    ("PH", "菲律宾"),
    ("ID", "印度尼西亚"),
    ("IN", "印度"),
    ("PK", "巴基斯坦"),
    ("BD", "孟加拉国"),
    ("KH", "柬埔寨"),
    ("KZ", "哈萨克斯坦"),
    ("AE", "阿联酋"),
    ("SA", "沙特阿拉伯"),
    ("IL", "以色列"),
    ("TR", "土耳其"),
    ("RU", "俄罗斯"),
    ("UA", "乌克兰"),
    ("US", "美国"),
    ("CA", "加拿大"),
    ("MX", "墨西哥"),
    ("BR", "巴西"),
    ("AR", "阿根廷"),
    ("CL", "智利"),
    ("CO", "哥伦比亚"),
    ("GB", "英国"),
    ("IE", "爱尔兰"),
    ("DE", "德国"),
    ("FR", "法国"),
    ("NL", "荷兰"),
    ("BE", "比利时"),
    ("LU", "卢森堡"),
    ("CH", "瑞士"),
    ("AT", "奥地利"),
    ("IT", "意大利"),
    ("ES", "西班牙"),
    ("PT", "葡萄牙"),
    ("SE", "瑞典"),
    ("NO", "挪威"),
    ("DK", "丹麦"),
    ("FI", "芬兰"),
    ("IS", "冰岛"),
    ("PL", "波兰"),
    ("CZ", "捷克"),
    ("SK", "斯洛伐克"),
    ("HU", "匈牙利"),
    ("RO", "罗马尼亚"),
    ("BG", "保加利亚"),
    ("GR", "希腊"),
    ("RS", "塞尔维亚"),
    ("HR", "克罗地亚"),
    ("SI", "斯洛文尼亚"),
    ("EE", "爱沙尼亚"),
    ("LV", "拉脱维亚"),
    ("LT", "立陶宛"),
    ("MD", "摩尔多瓦"),
    ("CY", "塞浦路斯"),
    ("MT", "马耳他"),
    ("ZA", "南非"),
    ("EG", "埃及"),
    ("NG", "尼日利亚"),
    ("KE", "肯尼亚"),
    ("AU", "澳大利亚"),
    ("NZ", "新西兰"),
];

pub fn region_name(code: &str) -> &str {
    REGIONS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
        .unwrap_or(code)
}

pub fn all() -> &'static [(&'static str, &'static str)] {
    REGIONS
}

/// Accepts a raw agent-reported code only if it looks like an alpha-2 code.
pub fn normalize(code: &str) -> Option<String> {
    let t = code.trim();
    if t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(t.to_ascii_uppercase())
    } else {
        None
    }
}
