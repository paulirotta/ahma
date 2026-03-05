use std::path::{Path, PathBuf};

fn encode_file_uri(path: &Path) -> String {
    let mut path_str = path.to_string_lossy().into_owned();
    if path_str.starts_with(r"\\?\") {
        path_str = path_str[4..].to_string();
    }
    path_str = path_str.replace('\\', "/");
    let mut out = String::with_capacity(path_str.len() + 10);
    out.push_str("file://");
    let is_drive = path_str.len() >= 2
        && path_str.as_bytes()[0].is_ascii_alphabetic()
        && path_str.as_bytes()[1] == b':';
    if is_drive {
        out.push('/');
    }
    for b in path_str.as_bytes() {
        let b = *b;
        let keep = matches!(
            b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':'
        );
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

fn parse_file_uri_to_path(uri: &str) -> Option<PathBuf> {
    const PREFIX: &str = "file://";
    if !uri.starts_with(PREFIX) {
        return None;
    }
    let mut rest = &uri[PREFIX.len()..];
    if let Some(idx) = rest.find(['?', '#']) {
        rest = &rest[..idx];
    }
    if let Some(after_localhost) = rest.strip_prefix("localhost") {
        rest = after_localhost;
    }
    let decoded = urlencoding::decode(rest).unwrap().into_owned();
    if let Some(without_leading_slash) = decoded.strip_prefix('/') {
        let is_drive = without_leading_slash.len() >= 2
            && without_leading_slash.as_bytes()[0].is_ascii_alphabetic()
            && without_leading_slash.as_bytes()[1] == b':';
        if is_drive {
            return Some(PathBuf::from(without_leading_slash));
        }
    }
    Some(PathBuf::from(decoded))
}

fn main() {
    let path = Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\.tmp123\root");
    let encoded = encode_file_uri(path);
    let encoded_with_space = encode_file_uri(Path::new(r"C:\Users\runner admin\AppData\Local\Temp\.tmp123\root"));
    println!("Encoded: {}", encoded);
    println!("Encoded (space): {}", encoded_with_space);
    
    let url = url::Url::parse(&encoded).unwrap();
    println!("url.to_file_path() = {:?}", url.to_file_path());
    
    let url_space = url::Url::parse(&encoded_with_space).unwrap();
    println!("url_space.to_file_path() = {:?}", url_space.to_file_path());
    
    // Test the empty URI: does rmcp fail?
    let json = serde_json::json!({
        "roots": [
            { "uri": "", "name": "empty" },
            { "uri": encoded, "name": "valid" }
        ]
    });
    // Can we deserialize ClientRoots? No rmcp access here.
}
