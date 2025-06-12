use reqwest::{Client, RequestBuilder};
use serde_json::json;

use crate::args::EvalArgs;

/// Posts the message to the PR on the github.
///
/// Updates an existing previous comment (if there is one) or posts a new comment.
pub async fn post_to_github_pr(
    args: &EvalArgs,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    const BASE_URL: &str = "https://api.github.com/repos/alpenlabs/strata";
    let comments_url = format!("{}/issues/{}/comments", BASE_URL, &args.pr_number);

    // Get all comments on the PR
    let comments_response = set_github_headers(client.get(&comments_url), &args.github_token)
        .send()
        .await?;

    let comments: Vec<serde_json::Value> = comments_response.json().await?;

    // Look for an existing comment from the bot
    let bot_comment = comments.iter().find(|comment| {
        comment["user"]["login"]
            .as_str()
            .map(|login| login == "github-actions[bot]")
            .unwrap_or(false)
    });

    // Depending on whether the bot has already commented, either patch or post
    let request = if let Some(existing_comment) = bot_comment {
        let comment_url = existing_comment["url"].as_str().unwrap();
        client.patch(comment_url)
    } else {
        client.post(&comments_url)
    };

    // Send the request with the updated or new body
    let response = set_github_headers(request, &args.github_token)
        .json(&json!({ "body": message }))
        .send()
        .await?;

    // Handle errors uniformly
    if !response.status().is_success() {
        let error_msg = if bot_comment.is_some() {
            "Failed to update comment"
        } else {
            "Failed to post comment"
        };
        return Err(format!("{}: {:?}", error_msg, response.text().await?).into());
    }

    Ok(())
}

// Helper function to apply common GitHub headers
fn set_github_headers(builder: RequestBuilder, token: &str) -> RequestBuilder {
    builder
        .header("Authorization", format!("Bearer {token}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "zkaleido-perf-bot")
}

pub fn format_github_message(results_text: &[String]) -> String {
    let mut formatted_message = String::new();

    for line in results_text {
        formatted_message.push_str(&line.replace('*', "**"));
        formatted_message.push('\n');
    }

    formatted_message
}
