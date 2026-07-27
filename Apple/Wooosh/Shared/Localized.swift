import Foundation

/// The one door every user-facing string goes through; copy lives in
/// `Localizable.xcstrings`, never in a view.
///
/// `t` and `f` are named apart rather than overloaded: an overload set would be
/// ambiguous with zero arguments and could ship unresolved specifiers. `f`
/// passes a non-nil locale so String Catalog plurals inflect in locales with
/// more than two plural forms.
enum L {

    static func t(_ key: String) -> String {
        NSLocalizedString(key, bundle: .main, comment: "")
    }

    static func f(_ key: String, _ arguments: any CVarArg...) -> String {
        String(format: NSLocalizedString(key, bundle: .main, comment: ""),
               locale: .current,
               arguments: arguments)
    }
}
