using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace Wooosh.ViewModels;

/// <summary>Partial because CsWinRT projects INotifyPropertyChanged onto a WinRT interface and generates the vtable alongside the class.</summary>
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
