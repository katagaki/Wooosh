using Windows.Storage;
using Wooosh.Core;

namespace Wooosh.Settings;

public sealed record WoooshSettings
{
    public required string DisplayName { get; init; }

    public required CoreVisibility Visibility { get; init; }

    /// <summary>On by default: a receiver that quits with its window is one that misses transfers.</summary>
    public required bool KeepRunningInBackground { get; init; }
}

/// <summary>
/// Three values, read once and written through. Anything that must survive a reinstall
/// (the identity key, the trust store) belongs to the core, not here.
/// </summary>
public sealed class SettingsRepository
{
    private const string DisplayNameKey = "displayName";
    private const string VisibilityKey = "visibility";
    private const string KeepRunningKey = "keepRunningInBackground";

    private readonly ApplicationDataContainer _store = ApplicationData.Current.LocalSettings;

    public event Action? Changed;

    public SettingsRepository()
    {
        Current = new WoooshSettings
        {
            DisplayName = _store.Values[DisplayNameKey] as string is { Length: > 0 } name
                ? name
                : Environment.MachineName,
            // Paired only by default: a fresh install must not accept strangers unopted-in.
            Visibility = (_store.Values[VisibilityKey] as string) switch
            {
                "everyone" => CoreVisibility.Everyone,
                "off" => CoreVisibility.Off,
                _ => CoreVisibility.PairedOnly,
            },
            KeepRunningInBackground = _store.Values[KeepRunningKey] as bool? ?? true,
        };
    }

    public WoooshSettings Current { get; private set; }

    public void SetDisplayName(string value)
    {
        var trimmed = value.Trim();
        if (trimmed.Length == 0 || trimmed == Current.DisplayName)
        {
            return;
        }

        _store.Values[DisplayNameKey] = trimmed;
        Current = Current with { DisplayName = trimmed };
        Changed?.Invoke();
    }

    public void SetVisibility(CoreVisibility value)
    {
        if (value == Current.Visibility)
        {
            return;
        }

        _store.Values[VisibilityKey] = value switch
        {
            CoreVisibility.PairedOnly => "pairedOnly",
            CoreVisibility.Off => "off",
            _ => "everyone",
        };
        Current = Current with { Visibility = value };
        Changed?.Invoke();
    }

    public void SetKeepRunningInBackground(bool value)
    {
        if (value == Current.KeepRunningInBackground)
        {
            return;
        }

        _store.Values[KeepRunningKey] = value;
        Current = Current with { KeepRunningInBackground = value };
        Changed?.Invoke();
    }
}
