# Wooosh — Copy & Localization Style Guide

Normative for all user-facing text in every shell (iOS, macOS, Android, and Windows when it exists). Applies to buttons, titles, section headers, body copy, empty states, error messages, notifications, share-sheet labels, permission-rationale strings, and accessibility labels.

## 1. Voice and tone
Professional and concise. Plain language a non-technical person understands on first read, without sounding curt.

- No jargon. Say "security code", not "SAS"; "connecting", not "handshake"; "device key changed", not "pin mismatch". Where a technical concept must surface (fingerprint verification), explain what to *do*, not what it *is*.
- Prefer the active voice and second person: "Choose a device to send to." not "A device must be chosen."
- Errors state what happened and what to try next, in that order, and never blame the user.
- Never expose internal identifiers, enum names, exception text, or file paths in copy meant for a normal user.

## 2. Product name
The product is **Wooosh**. Always refer to it by name.

- Never "this app", "the app", or "the application" in user-facing text.
- Correct: "Wooosh needs access to your local network to find nearby devices."
- Wrong: "This app needs access to your local network."
- The name is never translated or transliterated. It keeps its spelling in every locale, including CJK, and takes no article ("Wooosh", not "the Wooosh").

## 3. Punctuation
**No em-dashes (—) anywhere.** Rephrase rather than substituting a different dash.

- Split into two sentences, or use a colon, or restructure the clause.
- Wrong: "Wooosh could not reach that device — check your Wi-Fi."
- Right: "Wooosh could not reach that device. Check that both devices are on the same Wi-Fi network."
- En-dashes (–) are acceptable only in genuine numeric ranges. Hyphens in compound words are fine.

## 4. Capitalization
**Title Case** for buttons, titles, navigation titles, section headers, menu items, and tab labels: "Send Files", "Paired Devices", "Scan QR Code".

- Sentence case for body copy, subtitles, error messages, and helper text.
- Title Case is an **English-only rule.** Do not apply it to other locales. German capitalizes nouns by its own grammar; French, Spanish, Italian, Portuguese, Dutch, Polish, and Russian use sentence case for titles; CJK has no letter case at all. Blindly title-casing a translation is a localization bug.

## 5. Localization

Every user-facing string must be localizable. No hardcoded display strings in view code.

- **Apple**: a String Catalog (`.xcstrings`). Every string goes through it, including Info.plist usage descriptions (via `InfoPlist.xcstrings`) and share-extension copy.
- **Android**: `res/values/strings.xml` plus per-locale `res/values-<code>/strings.xml`. Compose must never inline a literal display string.

### Target locales
Base: **English (en)**.

CJK: **ja**, **ko**, **zh-Hans**, **zh-Hant**.

European: **de**, **es**, **fr**, **it**, **nl**, **pt-BR**, **pl**, **ru**, **sv**.

### Localization rules
- **Plurals use the platform's plural machinery** (String Catalog plural variations; Android `<plurals>`). Never build "1 file(s)" or concatenate a count with a hardcoded noun. Several target locales have more than two plural forms, so a two-branch English-style `if count == 1` is wrong.
- **Never concatenate sentence fragments.** Use one format string with positional placeholders (`%1$@` / `%1$s`) so translators can reorder freely. Word order differs sharply in Japanese, Korean, and German.
- **Do not translate**: the product name, device IDs, fingerprint phrases (they are a shared verification artifact and must read identically on both devices), file names, and protocol values.
- **Formatting comes from the system**: dates, times, byte sizes, and transfer rates use platform formatters, never hand-built strings.
- Every string carries a translator comment explaining context and, where relevant, the UI space available.
- Layouts must tolerate expansion. German and Russian commonly run 30 to 40 percent longer than English; CJK is shorter but needs correct line breaking. Buttons must not clip or truncate.
