// Пробник: сырые embeds/attachments последних сообщений по всем серверам —
// чтобы понять, что Discord отдаёт на ссылки вроде giphy/tenor.
// Запуск: cargo run --example embed_probe
fn read_token() -> String {
    let home = std::env::var("HOME").unwrap();
    let text =
        std::fs::read_to_string(format!("{home}/.config/discord-client/session.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["token"].as_str().unwrap().to_string()
}

#[tokio::main]
async fn main() {
    let token = read_token();
    let client = reqwest::Client::new();
    let get = |client: &reqwest::Client, token: &str, url: String| {
        client.get(url).header("Authorization", token).send()
    };

    let guilds: serde_json::Value = client
        .get("https://discord.com/api/v10/users/@me/guilds")
        .header("Authorization", &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let Some(garr) = guilds.as_array() else {
        println!("guilds — не массив: {guilds}");
        return;
    };

    let mut printed = 0usize;
    for g in garr {
        let gid = g["id"].as_str().unwrap_or("");
        let gname = g["name"].as_str().unwrap_or("");
        let channels: serde_json::Value = get(
            &client,
            &token,
            format!("https://discord.com/api/v10/guilds/{gid}/channels"),
        )
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        let Some(carr) = channels.as_array() else { continue };
        for ch in carr {
            let ch_id = ch["id"].as_str().unwrap_or("");
            let name = ch["name"].as_str().unwrap_or("");
            if ch["type"].as_i64() != Some(0) {
                continue;
            }
            let msgs: serde_json::Value = get(
                &client,
                &token,
                format!("https://discord.com/api/v10/channels/{ch_id}/messages?limit=50"),
            )
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            let Some(arr) = msgs.as_array() else { continue };
            for m in arr {
                let content = m["content"].as_str().unwrap_or("");
                let interesting = content.contains("giphy.com")
                    || content.contains("tenor.com")
                    ;
                for e in m["embeds"].as_array().cloned().unwrap_or_default() {
                    let eu = e["url"].as_str().unwrap_or("");
                    let et = e["type"].as_str().unwrap_or("");
                    if !(interesting
                        || eu.contains("giphy")
                        || eu.contains("tenor")
                        || matches!(et, "gifv" | "link" | "image" | "video" | "gif"))
                    {
                        continue;
                    }
                    printed += 1;
                    if printed > 12 {
                        return;
                    }
                    let short: String = content.chars().take(70).collect();
                    println!("=== [{gname} #{name}] content={short}");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&e).unwrap_or_default()
                    );
                }
            }
        }
    }
    if printed == 0 {
        println!("ничего giphy/tenor в последних 15 сообщениях серверов не нашлось");
    }
}
