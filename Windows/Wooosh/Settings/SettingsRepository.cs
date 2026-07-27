using Windows.Storage;
using Wooosh.Core;

namespace Wooosh.Settings;

public sealed record WoooshSettings
{
    /// <summary>What nearby devices see. Defaults to the machine name.</summary>
    public required string DisplayName { get; init; }

    public required CoreVisibility Visibility { get; init; }

    /// <summary>
    /// Closing the window leaves Wooosh in the notification area so it can keep receiving
    /// (DESIGN.md §7). On by default: a receiver that quits when its window is closed is a
    /// receiver that misses transfers.
    /// </summary>
    public required bool KeepRunningInBackground { get; init; }
}

/// <summary>
/// Settings, persisted in the packaged app's roaming-free local store.
///
/// Deliberately not a general-purpose settings framework: three values, read once, written
/// through. Anything that has to survive a reinstall (the identity key, the trust store)
/// belongs to the core, not here.
/// </summary>
public sealed class SettingsRepository
{
    private const string DisplayNameKey = "displayName";
    private const string VisibilityKey = "visibility";
    private const string KeepRunningKey = "keepRunningInBackground";

    private readonly ApplicationDataContainer _store = ApplicationData.Current.LocalSettings;

    /// <summary>Raised after any change, on the caller's thread.</summary>
    public event Action? Changed;

    public SettingsRepository()
    {
        Current = new WoooshSettings
        {
            DisplayName = _store.Values[DisplayNameKey] as string is { Length: > 0 } name
                ? name
                : Environment.MachineName,
            // Paired only by default: a fresh install should not accept transfers
            // from strangers on a shared network before the user has opted in.
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
