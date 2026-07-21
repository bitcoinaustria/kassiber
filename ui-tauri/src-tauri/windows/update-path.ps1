param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("add", "remove")]
    [string]$Action,

    [Parameter(Mandatory = $true)]
    [string]$Directory
)

$ErrorActionPreference = "Stop"

function Normalize-PathEntry([string]$Entry) {
    if ($null -eq $Entry) {
        return ""
    }
    return $Entry.Trim().TrimEnd([char[]]"\/")
}

$normalizedDirectory = Normalize-PathEntry $Directory
if ([string]::IsNullOrWhiteSpace($normalizedDirectory)) {
    throw "The Kassiber CLI directory must not be empty."
}

$currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
$entries = @()
if (-not [string]::IsNullOrWhiteSpace($currentPath)) {
    $entries = @($currentPath.Split(";", [StringSplitOptions]::RemoveEmptyEntries))
}

$kept = @($entries | Where-Object {
    -not [string]::Equals(
        (Normalize-PathEntry $_),
        $normalizedDirectory,
        [StringComparison]::OrdinalIgnoreCase
    )
})

if ($Action -eq "add") {
    $kept += $normalizedDirectory
}

$updatedPath = $kept -join ";"
[Environment]::SetEnvironmentVariable("Path", $updatedPath, "User")

# Tell Explorer and future child processes that the environment changed.
Add-Type -Namespace Kassiber -Name NativeMethods -MemberDefinition @"
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern IntPtr SendMessageTimeout(
        IntPtr hWnd,
        uint Msg,
        UIntPtr wParam,
        string lParam,
        uint fuFlags,
        uint uTimeout,
        out UIntPtr lpdwResult
    );
"@

$result = [UIntPtr]::Zero
[void][Kassiber.NativeMethods]::SendMessageTimeout(
    [IntPtr]0xffff,
    0x001A,
    [UIntPtr]::Zero,
    "Environment",
    0x0002,
    5000,
    [ref]$result
)
