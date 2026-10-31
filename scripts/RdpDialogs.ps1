# Shared by the RDP test scripts: accepts the control's certificate prompt for
# the test host only (clicks a matching button in a dialog of the given process).
if (-not ('RdpDialogs' -as [type])) {
Add-Type @"
using System; using System.Text; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class RdpDialogs {
  delegate bool Cb(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  static string Text(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
  static string Class(IntPtr h) { var s = new StringBuilder(64); GetClassName(h, s, 64); return s.ToString(); }
  // Clicks Yes/Connect in a visible dialog of the process. Returns "title | button" or null.
  public static string Accept(uint pid, string pattern) { return Accept(pid, pattern, false); }
  // Optionally ticks "Don't ask me again" first (Windows stores the certificate's CertHash).
  public static string Accept(uint pid, string pattern, bool remember) {
    string result = null;
    EnumWindows((h, l) => {
      uint owner; GetWindowThreadProcessId(h, out owner);
      if (owner != pid || !IsWindowVisible(h) || Class(h) != "#32770") return true;
      string title = Text(h);
      if (remember) EnumChildWindows(h, (c, l2) => {
        if (Class(c) == "Button" && System.Text.RegularExpressions.Regex.IsMatch(Text(c).Replace("&", ""), "(?i)ask me again|nicht erneut"))
          PostMessage(c, 0x00F1, (IntPtr)1, IntPtr.Zero); // BM_SETCHECK
        return true;
      }, IntPtr.Zero);
      EnumChildWindows(h, (c, l2) => {
        if (result == null && Class(c) == "Button" && System.Text.RegularExpressions.Regex.IsMatch(Text(c).Replace("&", ""), pattern)) {
          result = title + " | " + Text(c).Replace("&", "");
          PostMessage(c, 0x00F5, IntPtr.Zero, IntPtr.Zero); // BM_CLICK, never blocks on a hung window
        }
        return true;
      }, IntPtr.Zero);
      return result == null;
    }, IntPtr.Zero);
    return result;
  }
}
"@
}
