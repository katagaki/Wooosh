import Foundation

/// The one door every user-facing string goes through.
///
/// Copy lives in `Localizable.xcstrings`, never in a view. Two entry points,
/// deliberately named differently rather than overloaded: an overload set of
/// `f("key")` and `f("key", args…)` is ambiguous at the call site with zero
/// arguments, and the compiler picking the wrong one would silently ship a
/// format string with unresolved specifiers.
///
/// `f` passes a non-nil locale on purpose. That is what lets Foundation
/// resolve the `%#@…@` tokens a String Catalog plural compiles into, so
/// counted strings inflect correctly in the locales with more than two plural
/// forms (Polish and Russian have four).
enum L {

    /// A string with no arguments.
    static func t(_ key: String) -> String {
        NSLocalizedString(key, bundle: .main, comment: "")
    }

    /// A string with positional arguments, including plural-aware ones.
    static func f(_ key: String, _ arguments: any CVarArg...) -> String {
        String(format: NSLocalizedString(key, bundle: .main, comment: ""),
               locale: .current,
               arguments: arguments)
    }
}
