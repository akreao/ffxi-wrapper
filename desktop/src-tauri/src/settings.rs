//! Server settings changed from the app: LSB's settings from its
//! `main`, `map`, `login` and `search` files that the editor shows, kept in
//! `overrides/settings/app-settings.json` and written out as
//! `overrides/settings/zz_app.lua`, which LSB loads after every other
//! settings file there, so these win over the same keys in a hand-written
//! `map.lua` or `main.lua`. Only values that differ from LSB's defaults are
//! written.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone, Copy)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Kind {
    /// A number from `min` to `max`; `whole` ones take no fraction. The
    /// editor shows a slider from `min` to `slider` when `slider` is above
    /// `min` (the box still takes up to `max`), and `unit` after the value.
    Number { min: f64, max: f64, whole: bool, slider: f64, unit: &'static str },
    /// LSB's `true`/`false`.
    Bool,
    /// LSB's `1`/`0` on and off.
    Switch,
    /// One of these numbers, with what each means.
    Choice { options: &'static [(i64, &'static str)] },
}

#[derive(Serialize, Clone, Copy)]
pub struct Setting {
    /// The settings file it's in: `main`, `map`, `login` or `search`.
    pub file: &'static str,
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    #[serde(flatten)]
    pub kind: Kind,
    /// LSB's default (`settings/default/<file>.lua`).
    pub default: f64,
}

// The editor's sections, in this order.
const RATES: &str = "Rates";
const NEW: &str = "New characters";
const COMBAT: &str = "Combat and jobs";
const POWER: &str = "Spell and ability power";
const MOBS: &str = "Monsters and trusts";
const SKILLS: &str = "Skills and crafting";
const WORLD: &str = "World and travel";
const EXPANSIONS: &str = "Expansions";
const CONTENT: &str = "Content";
const QUESTS: &str = "Quests and battles";
const EVENTS: &str = "Events";
const AUCTION: &str = "Auction house";
const LOGINS: &str = "Logins and accounts";
const CHAT: &str = "Chat and moderation";

const X: &str = "×";
const SECONDS: &str = "seconds";
const HOURS: &str = "hours";

/// A multiplier: 1 is LSB's (retail's) value.
const fn rate(file: &'static str, key: &'static str, group: &'static str, label: &'static str, hint: &'static str) -> Setting {
    let kind = Kind::Number { min: 0.0, max: 100.0, whole: false, slider: 5.0, unit: X };
    Setting { file, key, group, label, hint, kind, default: 1.0 }
}

/// A multiplier kept from 0.1 to 2, as LSB asks for these.
const fn scale(key: &'static str, label: &'static str, hint: &'static str) -> Setting {
    let kind = Kind::Number { min: 0.1, max: 2.0, whole: false, slider: 2.0, unit: X };
    Setting { file: "map", key, group: MOBS, label, hint, kind, default: 1.0 }
}

/// A whole number; `slider` is 0 for none.
#[allow(clippy::too_many_arguments)]
const fn whole(
    file: &'static str,
    key: &'static str,
    group: &'static str,
    label: &'static str,
    hint: &'static str,
    (min, max, slider): (f64, f64, f64),
    unit: &'static str,
    default: f64,
) -> Setting {
    Setting { file, key, group, label, hint, kind: Kind::Number { min, max, whole: true, slider, unit }, default }
}

/// A number that takes fractions.
#[allow(clippy::too_many_arguments)]
const fn frac(
    file: &'static str,
    key: &'static str,
    group: &'static str,
    label: &'static str,
    hint: &'static str,
    (min, max, slider): (f64, f64, f64),
    unit: &'static str,
    default: f64,
) -> Setting {
    Setting { file, key, group, label, hint, kind: Kind::Number { min, max, whole: false, slider, unit }, default }
}

/// LSB's `true`/`false`.
const fn on(file: &'static str, key: &'static str, group: &'static str, label: &'static str, hint: &'static str, default: bool) -> Setting {
    Setting { file, key, group, label, hint, kind: Kind::Bool, default: if default { 1.0 } else { 0.0 } }
}

/// LSB's `1`/`0`.
const fn switch(file: &'static str, key: &'static str, group: &'static str, label: &'static str, hint: &'static str, default: bool) -> Setting {
    Setting { file, key, group, label, hint, kind: Kind::Switch, default: if default { 1.0 } else { 0.0 } }
}

const fn choice(
    file: &'static str,
    key: &'static str,
    group: &'static str,
    label: &'static str,
    hint: &'static str,
    options: &'static [(i64, &'static str)],
    default: f64,
) -> Setting {
    Setting { file, key, group, label, hint, kind: Kind::Choice { options }, default }
}

/// An expansion's content on or off (main.lua `ENABLE_*`).
const fn expansion(key: &'static str, label: &'static str) -> Setting {
    switch("main", key, EXPANSIONS, label, "", true)
}

const LEVEL: (f64, f64, f64) = (1.0, 99.0, 99.0);

pub const SETTINGS: &[Setting] = &[
    // Rates
    rate("map", "EXP_RATE", RATES, "Experience from monsters", "2 doubles it."),
    rate("main", "EXP_RATE", RATES, "Experience from quests", "Experience that quests and other scripts give, apart from Fields and Grounds of Valor."),
    rate("main", "BOOK_EXP_RATE", RATES, "Experience from Fields and Grounds of Valor", ""),
    rate("main", "ROE_EXP_RATE", RATES, "Experience from Records of Eminence", ""),
    rate("map", "CAPACITY_RATE", RATES, "Capacity points", ""),
    rate("map", "EXP_LOSS_RATE", RATES, "Experience lost on death", "0 loses none."),
    rate("map", "DROP_RATE_MULTIPLIER", RATES, "Item drop rate", ""),
    rate("map", "MOB_GIL_MULTIPLIER", RATES, "Gil from monsters", "The gil monsters drop on their own."),
    rate("main", "GIL_RATE", RATES, "Gil from quests", "The game doesn't always show the new amount."),
    rate("main", "BAYLD_RATE", RATES, "Bayld from quests", ""),
    rate("main", "TABS_RATE", RATES, "Tabs from Fields of Valor", ""),
    rate("main", "SPARKS_RATE", RATES, "Sparks from Records of Eminence", ""),
    rate("main", "SHOP_PRICE", RATES, "NPC shop prices", "0.5 halves every shop's prices."),
    frac("main", "CASKET_DROP_RATE", RATES, "Treasure casket chance", "0.1 is retail's 10%. 0 turns caskets off.", (0.0, 1.0, 1.0), "", 0.1),
    whole("map", "ALL_MOBS_GIL_BONUS", RATES, "Extra gil per monster level", "Every monster drops this much gil per level, even ones that drop none.", (0.0, 9999.0, 100.0), "gil", 0.0),
    whole("map", "MAX_GIL_BONUS", RATES, "Most extra gil per monster", "", (0.0, 999_999.0, 0.0), "gil", 9999.0),
    // New characters
    whole("main", "INITIAL_LEVEL_CAP", NEW, "Starting level cap", "Limit break quests raise it from here.", LEVEL, "", 50.0),
    whole("main", "MAX_LEVEL", NEW, "Highest level", "Lower it to stop the limit break quests going past it.", LEVEL, "", 99.0),
    whole("main", "START_GIL", NEW, "Starting gil", "", (0.0, 999_999_999.0, 0.0), "gil", 10.0),
    whole("main", "START_INVENTORY", NEW, "Starting inventory size", "Also the starting Mog Satchel size.", (30.0, 80.0, 80.0), "slots", 30.0),
    whole("main", "SUBJOB_QUEST_LEVEL", NEW, "Support job quest level", "0 starts every character with support jobs unlocked.", (0.0, 99.0, 99.0), "", 18.0),
    whole("main", "ADVANCED_JOB_LEVEL", NEW, "Advanced job quest level", "0 starts every character with the advanced jobs unlocked.", (0.0, 99.0, 99.0), "", 30.0),
    switch("main", "ALL_MAPS", NEW, "Start with every map", "", false),
    choice("main", "UNLOCK_OUTPOST_WARPS", NEW, "Outpost warps", "", &[(0, "Earned"), (1, "All"), (2, "All + Tu'Lia, Tavnazia")], 0.0),
    switch("main", "NEW_CHARACTER_CUTSCENE", NEW, "Opening cutscene", "The cutscene a new character sees when they first log in.", true),
    whole("main", "NEW_ADVENTURER_PLAYTIME_LIMIT", NEW, "New adventurer icon for", "Hours played before the icon goes. Retail is 240; it was 10 before The Voracious Resurgence.", (0.0, 10_000.0, 0.0), HOURS, 240.0),
    // Combat and jobs
    choice("map", "SUBJOB_RATIO", COMBAT, "Support job level", "How high a support job goes against the main job.", &[(0, "No support jobs"), (1, "Half (retail)"), (2, "Two thirds"), (3, "Equal")], 1.0),
    on("map", "INCLUDE_MOB_SJ", COMBAT, "Monsters' support jobs follow it too", "", false),
    frac("map", "SJ_MP_DIVISOR", COMBAT, "Support job MP divisor", "2 (retail) gives half the support job's MP.", (1.0, 10.0, 4.0), "", 2.0),
    on("map", "LEVEL_SYNC_ENABLE", COMBAT, "Level Sync", "", true),
    on("map", "DISABLE_GEAR_SCALING", COMBAT, "Block higher-level gear under a level cap", "Under a level cap or Level Sync, gear above the level can't be worn.", false),
    rate("map", "PLAYER_TP_MULTIPLIER", COMBAT, "Player TP gain", ""),
    rate("map", "PET_TP_MULTIPLIER", COMBAT, "Pet TP gain", "Includes charmed monsters."),
    rate("map", "ABILITY_RECAST_MULTIPLIER", COMBAT, "Job ability recast time", "0.5 halves every recast."),
    whole("map", "SPELL_RECAST_REDUCTION_CAP", COMBAT, "Spell recast reduction cap", "Retail is 80; older eras used 50.", (0.0, 100.0, 100.0), "%", 80.0),
    frac("main", "DELAY_REDUCTION_CAP", COMBAT, "Melee haste cap", "0.8 (80%) is retail; 0.93 is the Treasures of Aht Urhgan era.", (0.0, 1.0, 1.0), "", 0.8),
    whole("map", "HEALING_TICK_DELAY", COMBAT, "Time between resting ticks", "", (1.0, 60.0, 30.0), SECONDS, 10.0),
    whole("main", "HEALING_TP_CHANGE", COMBAT, "TP change per resting tick", "-100 (retail) loses 100 TP each tick.", (-3000.0, 3000.0, 0.0), "TP", -100.0),
    whole("map", "MAX_MERIT_POINTS", COMBAT, "Merit points held at once", "10 before Abyssea, 30 since.", (0.0, 99.0, 99.0), "", 30.0),
    on("map", "ALL_JOBS_WIDESCAN", COMBAT, "Wide Scan for every job", "Off keeps it to Beastmaster and Ranger.", true),
    on("map", "BLOOD_PACT_SHARED_TIMER", COMBAT, "One timer for both kinds of blood pact", "", false),
    on("map", "KEEP_JUGPET_THROUGH_ZONING", COMBAT, "Jug pets stay when zoning", "", false),
    on("main", "ENABLE_IMMUNOBREAK", COMBAT, "Immunobreaks", "", true),
    on("main", "USE_ADOULIN_WEAPON_SKILL_CHANGES", COMBAT, "Seekers of Adoulin weapon skill damage", "Off uses the older damage formulas.", true),
    // Spell and ability power
    rate("main", "CURE_POWER", POWER, "Healing magic", "Includes the healing Blue Magic."),
    rate("main", "ELEMENTAL_POWER", POWER, "Elemental and dark damage", "Dark Magic that doesn't drain."),
    rate("main", "DIVINE_POWER", POWER, "Divine magic", ""),
    rate("main", "NINJUTSU_POWER", POWER, "Ninjutsu damage", ""),
    rate("main", "BLUE_POWER", POWER, "Blue magic damage", ""),
    rate("main", "DARK_POWER", POWER, "Dark magic drains", ""),
    rate("main", "ITEM_POWER", POWER, "Potions and other items", ""),
    rate("main", "WEAPON_SKILL_POWER", POWER, "Weapon skill damage", ""),
    whole("main", "STONESKIN_CAP", POWER, "Stoneskin cap", "The most damage one Stoneskin soaks up.", (0.0, 9999.0, 0.0), "HP", 350.0),
    frac("main", "SNEAK_INVIS_DURATION_MULTIPLIER", POWER, "Sneak and Invisible duration", "Also Deodorize.", (0.0, 100.0, 5.0), X, 1.0),
    on("main", "USE_OLD_CURE_FORMULA", POWER, "Old cure formula", "Cure VI keeps the new one.", false),
    on("main", "USE_OLD_MAGIC_DAMAGE", POWER, "Old magic damage formulas", "", false),
    // Monsters and trusts
    scale("MOB_HP_MULTIPLIER", "Monster HP", ""),
    scale("NM_HP_MULTIPLIER", "Notorious monster HP", ""),
    scale("MOB_STAT_MULTIPLIER", "Monster stats", "Strength, vitality and the rest."),
    scale("NM_STAT_MULTIPLIER", "Notorious monster stats", ""),
    scale("ALTER_EGO_HP_MULTIPLIER", "Trust HP", "Also adventuring fellows."),
    scale("ALTER_EGO_STAT_MULTIPLIER", "Trust stats", ""),
    rate("map", "MOB_TP_MULTIPLIER", MOBS, "Monster TP gain", ""),
    rate("map", "TRUST_TP_MULTIPLIER", MOBS, "Trust TP gain", ""),
    frac("map", "MOB_RUN_SPEED_MULTIPLIER", MOBS, "Monster chase speed", "How much faster a monster runs when its target is out of reach. Retail is 2.5.", (0.0, 10.0, 5.0), X, 2.5),
    whole("map", "ENMITY_CAP", MOBS, "Enmity cap", "Lower lets damage dealers pull monsters off the tank sooner. Retail is about 30000.", (1.0, 1_000_000.0, 0.0), "", 30000.0),
    on("map", "MOB_NO_DESPAWN", MOBS, "Monsters walk home instead of vanishing", "", false),
    whole("map", "MOB_ADDITIONAL_TIME_TO_DEAGGRO", MOBS, "Extra time before a monster gives up", "Added to the usual 25 seconds.", (0.0, 600.0, 120.0), SECONDS, 0.0),
    frac("main", "NM_LOTTERY_CHANCE", MOBS, "Notorious monster spawn chance", "0 stops lottery spawns; -1 makes them always spawn.", (-1.0, 100.0, 5.0), X, 1.0),
    frac("main", "NM_LOTTERY_COOLDOWN", MOBS, "Notorious monster cooldown", "0 removes the wait between spawns.", (0.0, 100.0, 5.0), X, 1.0),
    whole("main", "FORCE_SPAWN_QM_RESET_TIME", MOBS, "??? hidden after its monster leaves", "", (0.0, 86_400.0, 0.0), SECONDS, 300.0),
    // Skills and crafting
    rate("map", "SKILLUP_CHANCE_MULTIPLIER", SKILLS, "Combat skill-up chance", ""),
    frac("map", "SKILLUP_AMOUNT_MULTIPLIER", SKILLS, "Combat skill-up size", "Above 1 lets one skill-up go past 0.5, up to 0.9. 5 is the most it does.", (0.0, 5.0, 5.0), X, 1.0),
    on("map", "SKILLUP_BLOODPACT", SKILLS, "Blood pacts raise summoning skill", "", true),
    on("map", "DEFENSIVE_OLD_SKILLUP_STYLE", SKILLS, "Parry, block and guard skill up on any hit", "Older eras only raised them when they happened.", false),
    rate("map", "CRAFT_CHANCE_MULTIPLIER", SKILLS, "Crafting skill-up chance", ""),
    rate("map", "CRAFT_HQ_CHANCE_MULTIPLIER", SKILLS, "Crafting high-quality chance", ""),
    on("map", "CRAFT_MODERN_SYSTEM", SKILLS, "Modern crafting skill-ups", "Retail's: more skill-ups, and up to 10 levels above a recipe.", true),
    whole("map", "CRAFT_COMMON_CAP", SKILLS, "Crafting level every craft can reach", "In tenths of a level: 700 (level 70) is retail, 600 the level 75 era.", (0.0, 1100.0, 0.0), "", 700.0),
    whole("map", "CRAFT_SPECIALIZATION_POINTS", SKILLS, "Specialization points above that", "Shared by all crafts. 3200 lets every craft reach the top.", (0.0, 3200.0, 3200.0), "", 400.0),
    on("map", "FISHING_ENABLE", SKILLS, "Fishing", "Off in LandSandBoat unless turned on, quests included.", false),
    whole("map", "FISHING_MIN_LEVEL", SKILLS, "Lowest level to fish", "", LEVEL, "", 1.0),
    rate("map", "FISHING_SKILL_MULTIPLIER", SKILLS, "Fishing skill-up chance", ""),
    on("main", "ENABLE_HELM_WAIT", SKILLS, "Wait between gathering tries", "3 seconds between harvesting, excavating, logging and mining.", false),
    whole("main", "DIG_FATIGUE", SKILLS, "Chocobo digs per day", "0 means no limit.", (0.0, 1000.0, 300.0), "", 100.0),
    // World and travel
    switch("main", "HOMEPOINT_TELEPORT", WORLD, "Home point teleports", "", true),
    whole("main", "EXPLORER_MOOGLE_LV", WORLD, "Explorer Moogle level", "The level a character needs for their teleports. 0 turns them off.", (0.0, 99.0, 99.0), "", 10.0),
    whole("map", "BASE_SPEED", WORLD, "Walking speed", "", (0.0, 255.0, 120.0), "", 50.0),
    whole("map", "SPEED_LIMIT", WORLD, "Top speed", "The most that gear and effects can raise it to.", (0.0, 255.0, 160.0), "", 80.0),
    whole("map", "MOUNT_SPEED", WORLD, "Chocobo and mount speed", "", (0.0, 255.0, 160.0), "", 80.0),
    on("map", "SELF_UNSTUCK_ENABLED", WORLD, "Unstuck from the Help Desk", "Players can send themselves to their home point.", false),
    whole("map", "SELF_UNSTUCK_COOLDOWN", WORLD, "Time between unstucks", "", (0.0, 604_800.0, 0.0), SECONDS, 86400.0),
    switch("main", "ENABLE_MOG_HOUSE_2F", WORLD, "Mog House second floor", "Also Mog Safe 2.", true),
    switch("main", "ENABLE_MOG_GARDEN", WORLD, "Mog Garden", "", true),
    on("map", "ENABLE_ITEM_RECYCLE_BIN", WORLD, "Recycle bin", "Off throws dropped items away at once.", true),
    on("main", "EQUIP_FROM_OTHER_CONTAINERS", WORLD, "Equip from Satchel, Sack and Case", "Needs a client addon.", false),
    on("main", "GUILD_SHOP_HOLIDAYS", WORLD, "Guild shops close on holidays", "", false),
    switch("main", "ENABLE_COP_ZONE_CAP", WORLD, "Level caps in Chains of Promathia areas", "", false),
    // Expansions
    switch("main", "RESTRICT_CONTENT", EXPANSIONS, "Leave out turned-off content fully", "Turned-off expansions' content isn't loaded at all, which is closer to playing without them.", false),
    expansion("ENABLE_ROTZ", "Rise of the Zilart"),
    expansion("ENABLE_COP", "Chains of Promathia"),
    expansion("ENABLE_TOAU", "Treasures of Aht Urhgan"),
    expansion("ENABLE_WOTG", "Wings of the Goddess"),
    expansion("ENABLE_ACP", "A Crystalline Prophecy"),
    expansion("ENABLE_AMK", "A Moogle Kupo d'Etat"),
    expansion("ENABLE_ASA", "A Shantotto Ascension"),
    expansion("ENABLE_ABYSSEA", "Abyssea"),
    expansion("ENABLE_SOA", "Seekers of Adoulin"),
    expansion("ENABLE_ROV", "Rhapsodies of Vana'diel"),
    expansion("ENABLE_TVR", "The Voracious Resurgence"),
    // Content
    switch("main", "ENABLE_FIELD_MANUALS", CONTENT, "Fields of Valor", "", true),
    switch("main", "ENABLE_GROUNDS_TOMES", CONTENT, "Grounds of Valor", "", true),
    switch("main", "REGIME_WAIT", CONTENT, "Valor pages wait for midnight", "As retail, a new page waits until 00:00 game time.", true),
    switch("main", "FOV_REWARD_ALLIANCE", CONTENT, "Fields of Valor rewards in an alliance", "", false),
    switch("main", "GOV_REWARD_ALLIANCE", CONTENT, "Grounds of Valor rewards in an alliance", "", true),
    switch("main", "ENABLE_ROE", CONTENT, "Records of Eminence", "", true),
    switch("main", "ENABLE_ROE_TIMED", CONTENT, "Timed records", "The 4-hour records.", true),
    switch("main", "ENABLE_EXCHANGE_LIMIT", CONTENT, "Weekly sparks limit", "", true),
    whole("main", "WEEKLY_EXCHANGE_LIMIT", CONTENT, "Sparks and accolades spent per week", "", (0.0, 9_999_999.0, 0.0), "", 100000.0),
    switch("main", "ENABLE_TRUST_CASTING", CONTENT, "Trusts", "Casting trust magic.", true),
    switch("main", "ENABLE_TRUST_QUESTS", CONTENT, "Trust quests", "", true),
    switch("main", "ENABLE_DAILY_TALLY", CONTENT, "Daily points", "For the Gobbie Mystery Box.", true),
    switch("main", "ENABLE_MAGIAN_TRIALS", CONTENT, "Magian trials", "", true),
    switch("main", "ENABLE_VOIDWALKER", CONTENT, "Voidwalker", "", true),
    switch("main", "ENABLE_MONSTROSITY", CONTENT, "Monstrosity", "", true),
    switch("main", "ENABLE_AMAN_TROVE", CONTENT, "A.M.A.N. Trove", "Greyson and the Mars and Venus Orb battlefields.", true),
    switch("main", "ENABLE_MMM", CONTENT, "Moblin Maze Mongers", "", false),
    on("main", "ENABLE_GARRISON", CONTENT, "Garrison", "", true),
    on("main", "NYZUL_ENABLED", CONTENT, "Nyzul Isle", "", false),
    on("main", "ENABLE_CHOCOBO_RAISING", CONTENT, "Chocobo raising", "", true),
    on("main", "ENABLE_CHOCOBO_RACING", CONTENT, "Chocobo racing", "", true),
    on("main", "ASSIST_CHANNEL_ENABLED", CONTENT, "Assist channel", "The chat channel for new and returning players.", true),
    // Quests and battles
    whole("main", "AF1_QUEST_LEVEL", QUESTS, "First artifact quest level", "", LEVEL, "", 40.0),
    whole("main", "AF2_QUEST_LEVEL", QUESTS, "Second artifact quest level", "", LEVEL, "", 50.0),
    whole("main", "AF3_QUEST_LEVEL", QUESTS, "Third artifact quest level", "", LEVEL, "", 50.0),
    on("main", "OLDSCHOOL_G1", QUESTS, "Old Genkai 1", "Farm the three drops instead of the key item way.", false),
    on("main", "OLDSCHOOL_G2", QUESTS, "Old Genkai 2", "The Atop the Highest Mountains monsters must be killed for the key items.", false),
    whole("main", "ASSAULT_MINIMUM", QUESTS, "Players needed for an assault", "The Treasures of Aht Urhgan era needed 3.", (1.0, 6.0, 6.0), "", 1.0),
    whole("map", "BATTLE_CAP_TWEAK", QUESTS, "Raise every battlefield level cap by", "Negative lowers them.", (-99.0, 99.0, 0.0), "levels", 0.0),
    on("map", "LV_CAP_MISSION_BCNM", QUESTS, "Level caps on mission battlefields", "", false),
    on("map", "BCNM_ENABLE_EXPERIMENTAL", QUESTS, "Unfinished battlefields", "Battlefields LandSandBoat marks as experimental.", true),
    whole("main", "BETWEEN_2DYNA_WAIT_TIME", QUESTS, "Wait between Dynamis runs", "", (0.0, 720.0, 72.0), HOURS, 24.0),
    whole("main", "DYNA_LEVEL_MIN", QUESTS, "Lowest level for Dynamis", "", LEVEL, "", 65.0),
    switch("main", "FREE_COP_DYNAMIS", QUESTS, "Chains of Promathia Dynamis without the missions", "", false),
    whole("main", "ENM_COOLDOWN", QUESTS, "Wait for the same ENM key item", "", (0.0, 1000.0, 0.0), HOURS, 120.0),
    on("main", "GARRISON_ONCE_PER_WEEK", QUESTS, "One garrison a week", "", true),
    on("main", "GARRISON_NATION_BYPASS", QUESTS, "Garrison for any nation", "", false),
    switch("main", "ALLOW_MULTIPLE_EXP_RINGS", QUESTS, "Own more than one experience ring", "The Chariot, Empress and Emperor Bands.", false),
    switch("main", "BYPASS_EXP_RING_ONE_PER_WEEK", QUESTS, "More than one experience ring a week", "", false),
    // Events
    switch("main", "HALLOWEEN_2005", EVENTS, "2005 Harvest Festival", "From 20 October to 1 November.", false),
    switch("main", "HALLOWEEN_YEAR_ROUND", EVENTS, "Harvest Festival all year", "", false),
    switch("main", "ENABLE_LOGIN_CAMPAIGN", EVENTS, "Login campaign", "Its dates are set in scripts/globals/events/login_campaign.lua.", false),
    switch("main", "ENABLE_STRANGE_HAPPENINGS", EVENTS, "Strange Happenings in Vana'diel", "", false),
    choice("main", "ENABLE_TRUST_ALTER_EGO_EXTRAVAGANZA", EVENTS, "Alter Ego Extravaganza", "", &[(0, "Off"), (1, "Summer, New Year"), (2, "Spring, autumn"), (3, "Both")], 0.0),
    switch("main", "ENABLE_TRUST_ALTER_EGO_EXTRAVAGANZA_ANNOUNCE", EVENTS, "Announce the Extravaganza at login", "", false),
    switch("main", "ENABLE_TRUST_ALTER_EGO_EXPO", EVENTS, "Alter Ego Expo", "Trusts get more HP, MP and status resistance.", false),
    switch("main", "ENABLE_TRUST_ALTER_EGO_EXPO_ANNOUNCE", EVENTS, "Announce the Expo at login", "", false),
    choice("main", "ENABLE_LIMITED_TIME_TRUST", EVENTS, "Limited-time trust", "Given to everyone with a trust permit.", &[(0, "None"), (1, "Cornelia"), (2, "Matsui-P")], 0.0),
    on("main", "AUTO_FISHING_CONTEST", EVENTS, "Fishing contest runs on its own", "Off needs a GM to move it along.", true),
    // Auction house
    whole("map", "AH_LIST_LIMIT", AUCTION, "Listings per player", "0 is no limit. Above 7 needs a client plugin.", (0.0, 100.0, 0.0), "", 7.0),
    whole("map", "AH_BASE_FEE_SINGLE", AUCTION, "Fee to list one item", "", (0.0, 1_000_000.0, 0.0), "gil", 1.0),
    whole("map", "AH_BASE_FEE_STACKS", AUCTION, "Fee to list a stack", "", (0.0, 1_000_000.0, 0.0), "gil", 4.0),
    frac("map", "AH_TAX_RATE_SINGLE", AUCTION, "Tax on one item", "Part of the price, added to the fee.", (0.0, 100.0, 10.0), "%", 1.0),
    frac("map", "AH_TAX_RATE_STACKS", AUCTION, "Tax on a stack", "", (0.0, 100.0, 10.0), "%", 0.5),
    whole("map", "AH_MAX_FEE", AUCTION, "Highest fee", "", (0.0, 999_999_999.0, 0.0), "gil", 10000.0),
    on("search", "EXPIRE_AUCTIONS", AUCTION, "Listings expire", "", true),
    whole("search", "EXPIRE_DAYS", AUCTION, "Listings expire after", "", (1.0, 365.0, 30.0), "days", 3.0),
    on("search", "OMIT_NO_HISTORY", AUCTION, "Hide items never sold", "Leave items with no sales history out of the auction house lists.", false),
    // Logins and accounts
    switch("login", "MAINT_MODE", LOGINS, "Maintenance mode", "Only GM characters can log in, and no new characters can be made.", false),
    on("login", "ACCOUNT_CREATION", LOGINS, "New accounts", "Players can make an account from the game's login.", true),
    on("login", "CHARACTER_CREATION", LOGINS, "New characters", "", true),
    on("login", "CHARACTER_DELETION", LOGINS, "Character deletion", "", true),
    whole("login", "LOGIN_LIMIT", LOGINS, "Games per address", "How many can play from one IP address at once. 0 is no limit.", (0.0, 100.0, 0.0), "", 0.0),
    on("login", "DISABLE_MOB_NPC_CHAR_NAMES", LOGINS, "Block monster and NPC names", "Characters can't be called Shantotto, Fafnir and so on.", false),
    on("login", "LOG_USER_IP", LOGINS, "Keep players' IP addresses", "Saved in the database.", false),
    // Chat and moderation
    whole("map", "YELL_COOLDOWN", CHAT, "Time between yells", "", (0.0, 3600.0, 120.0), SECONDS, 30.0),
    on("map", "BLOCK_TELL_TO_HIDDEN_GM", CHAT, "Block tells to hidden GMs", "Other GMs can still send them.", false),
    on("map", "AUDIT_CHAT", CHAT, "Log chat to the database", "", false),
    on("map", "AUDIT_PLAYER_TRADES", CHAT, "Log trades between players", "", false),
    on("map", "AUDIT_PLAYER_BAZAAR", CHAT, "Log bazaar sales", "", false),
    on("map", "AUDIT_PLAYER_DBOX", CHAT, "Log delivery box sends", "", false),
    on("map", "PREVENT_UNENGAGED_WS", CHAT, "Block weapon skills when not fighting", "Stops a cheat that sends them with packets.", false),
    on("map", "HIDE_READIES_TARGET", CHAT, "Hide who monsters are targeting", "Stops addons showing the target of a monster's spell or move.", false),
];

/// Values by `file.KEY`, as numbers (`1`/`0` for on and off).
pub type Values = BTreeMap<String, f64>;

fn id(s: &Setting) -> String {
    format!("{}.{}", s.file, s.key)
}

fn json_path(stack: &Path) -> PathBuf {
    stack.join("overrides").join("settings").join("app-settings.json")
}

pub fn lua_path(stack: &Path) -> PathBuf {
    stack.join("overrides").join("settings").join("zz_app.lua")
}

/// The saved values, with LSB's default for each one not set.
pub fn load(stack: &Path) -> Values {
    let saved: BTreeMap<String, Value> = fs::read(json_path(stack))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    SETTINGS
        .iter()
        .map(|s| {
            let value = saved.get(&id(s)).and_then(Value::as_f64).filter(|v| check(s, *v).is_ok());
            (id(s), value.unwrap_or(s.default))
        })
        .collect()
}

fn check(s: &Setting, v: f64) -> Result<(), String> {
    let ok = v.is_finite()
        && match s.kind {
            Kind::Number { min, max, whole, .. } => v >= min && v <= max && (!whole || v.fract() == 0.0),
            Kind::Bool | Kind::Switch => v == 0.0 || v == 1.0,
            Kind::Choice { options } => options.iter().any(|(o, _)| *o as f64 == v),
        };
    if ok {
        return Ok(());
    }
    Err(match s.kind {
        Kind::Number { min, max, whole: true, .. } => format!("{} must be a whole number from {min} to {max}.", s.label),
        Kind::Number { min, max, .. } => format!("{} must be from {min} to {max}.", s.label),
        _ => format!("{} has a value it can't take.", s.label),
    })
}

/// Saves `values` (missing ones keep their saved value) and writes the
/// Lua file. Returns whether anything changed.
pub fn save(stack: &Path, values: &Values) -> Result<bool, String> {
    let mut all = load(stack);
    for s in SETTINGS {
        if let Some(v) = values.get(&id(s)) {
            check(s, *v)?;
            all.insert(id(s), *v);
        }
    }
    if all.get("main.INITIAL_LEVEL_CAP") > all.get("main.MAX_LEVEL") {
        return Err("The starting level cap can't be above the highest level.".into());
    }
    let changed: Values = SETTINGS
        .iter()
        .filter(|s| all[&id(s)] != s.default)
        .map(|s| (id(s), all[&id(s)]))
        .collect();
    let lua = lua(&changed);
    let before = fs::read_to_string(lua_path(stack)).ok();
    let dir = json_path(stack).parent().unwrap().to_owned();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&changed).map_err(|e| e.to_string())?;
    fs::write(json_path(stack), json).map_err(|e| e.to_string())?;
    if changed.is_empty() {
        let _ = fs::remove_file(lua_path(stack));
    } else {
        fs::write(lua_path(stack), &lua).map_err(|e| e.to_string())?;
    }
    let after = (!changed.is_empty()).then_some(lua);
    Ok(before != after)
}

fn lua(changed: &Values) -> String {
    let mut out = String::from(
        "-- Written by the ffxi-wrapper desktop app from its Game settings, and\n\
         -- replaced when they're saved: change these there. LSB loads this file\n\
         -- after the other settings files here, so its values win.\n\
         xi = xi or {}\n\
         xi.settings = xi.settings or {}\n",
    );
    let mut files: Vec<&str> = SETTINGS.iter().map(|s| s.file).collect();
    files.dedup();
    files.sort();
    files.dedup();
    for file in files {
        let mine: Vec<&Setting> = SETTINGS.iter().filter(|s| s.file == file && changed.contains_key(&id(s))).collect();
        if mine.is_empty() {
            continue;
        }
        out.push_str(&format!("xi.settings.{file} = xi.settings.{file} or {{}}\n"));
        for s in mine {
            let v = changed[&id(s)];
            let value = match s.kind {
                Kind::Bool => (if v == 1.0 { "true" } else { "false" }).to_owned(),
                _ => v.to_string(),
            };
            out.push_str(&format!("xi.settings.{file}.{} = {value}\n", s.key));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_only_what_differs_from_lsb() {
        let stack = temp("write");
        assert_eq!(load(&stack)["map.EXP_RATE"], 1.0);
        let mut v = Values::new();
        v.insert("map.EXP_RATE".into(), 2.5);
        v.insert("map.FISHING_ENABLE".into(), 1.0);
        v.insert("main.START_GIL".into(), 10.0);
        v.insert("main.ALL_MAPS".into(), 1.0);
        assert!(save(&stack, &v).unwrap());
        let lua = fs::read_to_string(lua_path(&stack)).unwrap();
        assert!(lua.contains("xi.settings.map.EXP_RATE = 2.5\n"));
        assert!(lua.contains("xi.settings.map.FISHING_ENABLE = true\n"));
        assert!(lua.contains("xi.settings.main.ALL_MAPS = 1\n"));
        assert!(!lua.contains("START_GIL"));
        assert!(lua.find("xi.settings.main =").unwrap() < lua.find("xi.settings.map =").unwrap());
        assert_eq!(load(&stack)["map.EXP_RATE"], 2.5);
        // Saving the same again changes nothing.
        assert!(!save(&stack, &v).unwrap());

        // Back to the defaults: the file goes.
        let mut back = Values::new();
        back.insert("map.EXP_RATE".into(), 1.0);
        back.insert("map.FISHING_ENABLE".into(), 0.0);
        back.insert("main.ALL_MAPS".into(), 0.0);
        assert!(save(&stack, &back).unwrap());
        assert!(!lua_path(&stack).exists());
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn refuses_values_out_of_range() {
        let stack = temp("range");
        let one = |k: &str, v: f64| Values::from([(k.to_owned(), v)]);
        assert!(save(&stack, &one("main.START_INVENTORY", 81.0)).is_err());
        assert!(save(&stack, &one("main.MAX_LEVEL", 75.5)).is_err());
        assert!(save(&stack, &one("map.EXP_RATE", -1.0)).is_err());
        assert!(save(&stack, &one("main.UNLOCK_OUTPOST_WARPS", 3.0)).is_err());
        assert!(save(&stack, &one("map.FISHING_ENABLE", 2.0)).is_err());
        assert!(save(&stack, &one("main.MAX_LEVEL", 40.0)).unwrap_err().contains("level cap"));
        assert!(save(&stack, &one("main.UNLOCK_OUTPOST_WARPS", 2.0)).is_ok());
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn every_setting_has_a_valid_default() {
        for s in SETTINGS {
            assert!(check(s, s.default).is_ok(), "{}", id(s));
            if let Kind::Number { min, max, slider, .. } = s.kind {
                assert!(slider == 0.0 || (slider > min && slider <= max), "{} slider", id(s));
            }
        }
    }

    #[test]
    fn settings_are_listed_once_and_grouped_together() {
        let mut seen = std::collections::BTreeSet::new();
        let mut groups: Vec<&str> = Vec::new();
        for s in SETTINGS {
            assert!(seen.insert(id(s)), "{} twice", id(s));
            assert!(["main", "map", "login", "search"].contains(&s.file), "{}", id(s));
            if groups.last() != Some(&s.group) {
                assert!(!groups.contains(&s.group), "{} split up", s.group);
                groups.push(s.group);
            }
        }
    }
}
