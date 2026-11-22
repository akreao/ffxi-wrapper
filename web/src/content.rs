//! The blocks of text the owner can edit under "Site content", and what each says
//! until it has been edited.

use crate::config::Config;
use crate::markdown;
use crate::store::ContentRow;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Block {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// The most bytes it may hold.
    pub max: usize,
    /// Plain text on one line (the title), not Markdown.
    pub plain: bool,
}

pub const BLOCKS: [Block; 6] = [
    Block { key: "home_title", label: "Home page title", hint: "The big heading on the home page. Plain text; the server's name until you change it.", max: 80, plain: true },
    Block { key: "home_description", label: "Home page description", hint: "A few lines under the title.", max: 1_000, plain: false },
    Block { key: "connect", label: "How to connect", hint: "The steps in the box next to the title. The server's address, when the site knows it, is shown below your text with a Copy button.", max: 2_000, plain: false },
    Block { key: "rules", label: "House rules", hint: "The text of the Rules page.", max: 20_000, plain: false },
    Block { key: "bots", label: "AI adventurers", hint: "Shown on the Rules page, and linked from Who's online, only while the server has AI adventurers (player bots) switched on.", max: 5_000, plain: false },
    Block { key: "footer", label: "Footer", hint: "The line at the bottom of every public page.", max: 400, plain: false },
];

/// What players are told about LSB's player bots until the owner edits it.
pub const BOTS_NOTICE: &str = "**AI adventurers.** Some of the adventurers you meet on this server are run by the server, not by people. \
They hunt, travel, rest in town, say a few lines, and can join your party, where they take a share of experience like a trust. \
They show up in /sea like anyone else. Their /check comment reads \"AI adventurer\". \
They never trade, bid at the auction house, or join linkshells, and nothing about them is saved. \
If you'd rather not have them around, type `!bots off` in game or use the switch on [your character's page](/me) on this site. \
They are then hidden from you, their chat doesn't reach you, and they won't hunt near you. `!bots on` brings them back.";

pub fn block(key: &str) -> Option<&'static Block> {
    BLOCKS.iter().find(|b| b.key == key)
}

/// The address players put in their client: the site's own host name when it is set.
pub fn connect_address(cfg: &Config) -> Option<String> {
    markdown::host_of(&cfg.public_url)
}

pub fn default_body(key: &str, cfg: &Config) -> String {
    match key {
        "home_title" => cfg.server_name.clone(),
        "home_description" => "A LandSandBoat server. Sign in with the login and password you play with to see your characters.".into(),
        "connect" => {
            let address = if connect_address(cfg).is_some() {
                "the address below"
            } else {
                "the server's address (ask the server owner for it)"
            };
            format!(
                "1. Get [vana-client](https://github.com/lenonk/vana-client) and the game data.\n2. Set `\"server\"` in its `config.json` to {address}.\n3. Log in with the account the server owner made for you."
            )
        }
        "rules" => "The server owner hasn't written the house rules yet.".into(),
        "bots" => BOTS_NOTICE.into(),
        "footer" => format!("{} runs on [LandSandBoat](https://github.com/LandSandBoat/server). Not affiliated with Square Enix.", cfg.server_name),
        _ => String::new(),
    }
}

/// What a block says now: what was saved, else its default. The flag is true for the default.
pub fn effective(key: &str, stored: &[ContentRow], cfg: &Config) -> (String, bool) {
    match stored.iter().find(|r| r.key == key) {
        Some(r) => (r.body.clone(), false),
        None => (default_body(key, cfg), true),
    }
}

/// Is `text` fit to be stored in `block`? The message is for the editor.
pub fn check(block: &Block, text: &str) -> Result<(), String> {
    if text.len() > block.max {
        return Err(format!("{} can hold at most {} characters.", block.label, block.max));
    }
    if block.plain {
        if text.trim().is_empty() {
            return Err(format!("{} can't be empty. Reset it to use the server's name.", block.label));
        }
        if text.chars().any(|c| c.is_control()) {
            return Err(format!("{} must be one line of plain text.", block.label));
        }
    } else if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("The text contains control characters.".into());
    }
    Ok(())
}
