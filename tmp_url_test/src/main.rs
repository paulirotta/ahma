fn main() {
    let json_val = serde_json::json!({
        "roots": [
            { "uri": "", "name": "root" },
            { "uri": "ftp://also-invalid/path", "name": "root2" },
            { "uri": "http://invalid/not-file-scheme", "name": "root3" },
            { "uri": "file:///C:/Users/runneradmin/root", "name": "root4" }
        ]
    });
    match serde_json::from_value::<rmcp::types::ClientRoots>(json_val) {
        Ok(cr) => println!("Success! roots length is {}", cr.roots.len()),
        Err(e) => println!("Failed to deserialize: {}", e)
    }
}
