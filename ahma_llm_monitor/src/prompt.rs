/// Build the OpenAI-style messages array for a log detection request.
///
/// The user-supplied `detection_prompt` describes what to look for in plain English.
/// The `chunk` contains the recent log lines to analyse.
///
/// The LLM is instructed to respond with exactly one word `CLEAN` if there are no
/// issues, or a brief plain-English summary (1–3 sentences) if issues are found.
pub fn build_messages(detection_prompt: &str, chunk: &str) -> Vec<serde_json::Value> {
    use serde_json::json;

    let system = "You are a log analysis assistant. \
You will be given recent log output and a description of what to look for. \
Analyse the logs and respond with EITHER:\n\
- The single word CLEAN if there are no issues matching the description\n\
- A brief plain-English summary (1-3 sentences) describing the detected issue \
and the most relevant log lines if issues ARE found. \
Do NOT include any other text, preamble, or explanation.";

    let user =
        format!("Detection criteria:\n{detection_prompt}\n\nRecent log output:\n```\n{chunk}\n```");

    vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user", "content": user}),
    ]
}
