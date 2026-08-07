namespace TealTeam.Web.Views;

/// <summary>Formatting helpers matching the Go template funcMap (float1, float2, optionLabel).</summary>
public static class ViewHelpers
{
    public static string Float1(double? v) => (v ?? 0).ToString("0.0");
    public static string Float2(double? v) => (v ?? 0).ToString("0.00");

    /// <summary>"hang_level" -> "Hang Level", "l1" -> "L1"</summary>
    public static string OptionLabel(string optionKey)
        => string.Join(" ", optionKey.Split('_')
            .Where(p => p != "")
            .Select(p => char.ToUpperInvariant(p[0]) + p[1..]));

    /// <summary>Title-case a single option value for display ("fast" -> "Fast").</summary>
    public static string Title(string? value)
        => string.IsNullOrEmpty(value) ? "" : char.ToUpperInvariant(value[0]) + value[1..];

    public static string ValueOrNA(string? s) => string.IsNullOrEmpty(s) ? "Unknown" : s;
}
