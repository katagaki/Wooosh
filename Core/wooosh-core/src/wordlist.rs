//! 256 words from the EFF short wordlist; one fingerprint byte indexes one word.

pub const WORDS: [&str; 256] = [
    "acid", "acorn", "acre", "acts", "afar", "affix", "aged", "agent",
    "agile", "aging", "agony", "ahead", "aide", "aids", "aim", "ajar",
    "alarm", "alias", "alibi", "alien", "alike", "alive", "aloe", "aloft",
    "aloha", "alone", "amend", "amino", "ample", "amuse", "angel", "anger",
    "angle", "ankle", "apple", "april", "apron", "aqua", "area", "arena",
    "argue", "arise", "armed", "armor", "army", "aroma", "array", "arson",
    "art", "ashen", "ashes", "atlas", "atom", "attic", "audio", "avert",
    "avoid", "awake", "award", "awoke", "axis", "bacon", "badge", "bagel",
    "baggy", "baked", "baker", "balmy", "banjo", "barge", "barn", "batch",
    "bath", "baton", "bats", "blade", "blank", "blast", "blaze", "bleak",
    "blend", "bless", "blimp", "blink", "bloat", "blob", "blog", "blot",
    "blunt", "blurt", "blush", "boast", "boat", "body", "boil", "bok",
    "bolt", "boned", "boney", "bonus", "bony", "book", "booth", "boots",
    "boss", "botch", "both", "boxer", "breed", "bribe", "brick", "bride",
    "brim", "bring", "brink", "brisk", "broad", "broil", "broke", "brook",
    "broom", "brush", "buck", "bud", "buggy", "bulge", "bulk", "bully",
    "bunch", "bunny", "bunt", "bush", "bust", "busy", "buzz", "cable",
    "cache", "cadet", "cage", "cake", "calm", "cameo", "canal", "candy",
    "cane", "canon", "cape", "card", "cargo", "carol", "carry", "carve",
    "case", "cash", "cause", "cedar", "chain", "chair", "chant", "chaos",
    "charm", "chase", "cheek", "cheer", "chef", "chess", "chest", "chew",
    "chief", "chili", "chill", "chip", "chomp", "chop", "chow", "chuck",
    "chump", "chunk", "churn", "chute", "cider", "cinch", "city", "civic",
    "civil", "clad", "claim", "clamp", "clap", "clash", "clasp", "class",
    "claw", "clay", "clean", "clear", "cleat", "cleft", "clerk", "click",
    "cling", "clink", "clip", "cloak", "clock", "clone", "cloth", "cloud",
    "clump", "coach", "coast", "coat", "cod", "coil", "coke", "cola",
    "cold", "colt", "coma", "come", "comic", "comma", "cone", "cope",
    "copy", "coral", "cork", "cost", "cot", "couch", "cough", "cover",
    "cozy", "craft", "cramp", "crane", "crank", "crate", "crave", "crawl",
    "crazy", "creme", "crepe", "crept", "crib", "cried", "crisp", "crook",
    "crop", "cross", "crowd", "crown", "crumb", "crush", "crust", "cub",
];

pub fn phrase(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| WORDS[*b as usize])
        .collect::<Vec<_>>()
        .join(" ")
}
