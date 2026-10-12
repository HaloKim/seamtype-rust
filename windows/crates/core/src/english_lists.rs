// Word lists of EnglishModel.swift, kept here so build.rs (which bakes the word table) and the
// library read the same text. Keep in step with Sources/SeamtypeCore/EnglishModel.swift.

/// Many 1–2 letter list entries aren't words people type ("ra", "ko"); only these count.
pub const SHORT_WORDS: &str = "
    a i ad ah am an as at aw ax be by do eh go ha he hi hm if in is it lo ma me mo my no of oh ok on or ow ox pa
    pc so to tv uh um up us we ya ye yo id ex vs ai ui ux os js ts
";

/// Everyday/technical words SCOWL is missing or ranks rare.
pub const COMMON_WORDS: &str = "
    ok okay hi hey yeah yep nope lol omg btw thx thanks pls please email emails app apps api apis url urls
    http https www github git commit push pull merge branch repo json yaml xml html css js ts swift python java
    kotlin rust golang npm yarn pnpm node react vue svelte ios macos android iphone ipad mac wifi bluetooth usb
    login logout signup username password online offline website blog vlog youtube google slack notion figma
    debug deploy refactor config backend frontend database cache server client async await todo todos ai llm gpt
    claude chatgpt laptop smartphone selfie emoji internet podcast startup dev devs devops ui ux
    wanna gonna gotta kinda sorta outta lemme gimme dunno yeah yup nah ain cuz tho thru hiya kiddo sayin
    goin doin nothin somethin y'all ya yo hey oi aye
    wouldn couldn shouldn didn doesn isn wasn weren hasn haven hadn needn mustn aren mightn
    makeup brooklyn shakespeare einstein techno pesto americano cosmo promo garde kanji kana romaji ramen
";

/// Level given to the common words (tech terms and slang SCOWL lacks): known, not "common".
pub const EXTRA_LEVEL: i64 = 35;
