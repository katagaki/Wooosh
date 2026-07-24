using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace Wooosh.ViewModels;

/// <summary>
/// Minimal INotifyPropertyChanged base.
///
/// Partial because INotifyPropertyChanged is projected onto a WinRT interface, and CsWinRT
/// wants such classes declared partial so it can generate the vtable alongside them.
///
/// Hand-rolled rather than taken from the MVVM Toolkit: the Windows shell needs exactly
/// this, and a source-generator dependency on a project that also has to line up a Rust
/// DLL, an MSIX manifest and a WinAppSDK version is one moving part too many.
/// </summary>
public abstract partial class ObservableObject : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    protected bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        Raise(name);
        return true;
    }

    protected void Raise([CallerMemberName] string? name = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
