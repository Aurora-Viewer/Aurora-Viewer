# Visual marker for unattended viewer tests on Windows.
# Run with Windows PowerShell in STA mode, in the viewer's desktop session.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 2147483647)]
    [int]$ViewerProcessId
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace AuroraAutomation
{
    internal static class Native
    {
        internal delegate bool EnumWindowCallback(IntPtr window, IntPtr parameter);
        [StructLayout(LayoutKind.Sequential)]
        internal struct Rect { public int Left, Top, Right, Bottom; }
        [DllImport("user32.dll")] internal static extern bool EnumWindows(EnumWindowCallback callback, IntPtr parameter);
        [DllImport("user32.dll")] internal static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] internal static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
        [DllImport("user32.dll")] internal static extern bool IsWindowVisible(IntPtr window);
        [DllImport("user32.dll")] internal static extern bool IsIconic(IntPtr window);
        [DllImport("user32.dll")] internal static extern bool GetWindowRect(IntPtr window, out Rect rectangle);
        [DllImport("user32.dll")] internal static extern IntPtr GetWindow(IntPtr window, uint command);
        [DllImport("user32.dll")] internal static extern int GetWindowLong(IntPtr window, int index);
        [DllImport("user32.dll")] internal static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
        [DllImport("user32.dll")] internal static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("dwmapi.dll", EntryPoint = "DwmGetWindowAttribute")] internal static extern int GetFrame(IntPtr window, int attribute, out Rect value, int size);
        [DllImport("dwmapi.dll", EntryPoint = "DwmGetWindowAttribute")] internal static extern int GetCloaked(IntPtr window, int attribute, out int value, int size);

        internal static bool IsMarked(IntPtr window, int processId)
        {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != (uint)processId || !IsWindowVisible(window) || IsIconic(window)) return false;
            int cloaked;
            if (GetCloaked(window, 14, out cloaked, sizeof(int)) == 0 && cloaked != 0) return false;
            var title = new StringBuilder(1024);
            GetWindowText(window, title, title.Capacity);
            return title.ToString().IndexOf("[AUTOMATION - DO NOT TOUCH]", StringComparison.Ordinal) >= 0;
        }
    }

    internal sealed class Border : Form
    {
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override CreateParams CreateParams
        {
            get
            {
                var parameters = base.CreateParams;
                // Layered + transparent passes clicks through, NOACTIVATE preserves focus.
                parameters.ExStyle |= 0x00080000 | 0x00000020 | 0x08000000 | 0x00000080;
                return parameters;
            }
        }

        internal Border()
        {
            Text = "Aurora automation border";
            FormBorderStyle = FormBorderStyle.None;
            ShowInTaskbar = false;
            StartPosition = FormStartPosition.Manual;
            AutoScaleMode = AutoScaleMode.None;
            BackColor = Color.Magenta;
            TransparencyKey = Color.Magenta;
            DoubleBuffered = true;
            SetStyle(ControlStyles.ResizeRedraw, true);
        }

        protected override void OnPaint(PaintEventArgs arguments)
        {
            base.OnPaint(arguments);
            // Inset keeps the entire stroke visible when the viewer is maximized.
            using (var pen = new Pen(Color.FromArgb(255, 215, 0), 4))
            {
                pen.DashStyle = DashStyle.Dash;
                arguments.Graphics.DrawRectangle(pen, 2, 2, Math.Max(0, ClientSize.Width - 5), Math.Max(0, ClientSize.Height - 5));
            }
        }

        internal void Follow(IntPtr target)
        {
            Native.Rect frame;
            if (Native.GetFrame(target, 9, out frame, Marshal.SizeOf(typeof(Native.Rect))) != 0
                && !Native.GetWindowRect(target, out frame))
            {
                Hide();
                return;
            }
            int width = frame.Right - frame.Left, height = frame.Bottom - frame.Top;
            if (width <= 0 || height <= 0) { Hide(); return; }

            // Insert immediately above the target, so unrelated foreground windows cover us.
            IntPtr preceding = Native.GetWindow(target, 3); // GW_HWNDPREV
            uint flags = 0x0010; // SWP_NOACTIVATE
            if (preceding == Handle) flags |= 0x0004; // SWP_NOZORDER
            else if ((Native.GetWindowLong(target, -20) & 0x00000008) == 0
                && preceding != IntPtr.Zero && (Native.GetWindowLong(preceding, -20) & 0x00000008) != 0)
                preceding = IntPtr.Zero; // HWND_TOP, below the topmost group

            if (!Visible)
            {
                // Position while hidden to avoid a flash at the default location.
                Native.SetWindowPos(Handle, preceding, frame.Left, frame.Top, width, height, flags);
                Show();
                preceding = Native.GetWindow(target, 3);
                if (preceding == Handle) flags |= 0x0004;
                else if ((Native.GetWindowLong(target, -20) & 0x00000008) == 0
                    && preceding != IntPtr.Zero && (Native.GetWindowLong(preceding, -20) & 0x00000008) != 0)
                    preceding = IntPtr.Zero;
            }
            Native.SetWindowPos(Handle, preceding, frame.Left, frame.Top, width, height, flags);
        }
    }

    internal sealed class BorderSession : ApplicationContext
    {
        private readonly Process target;
        private readonly Dictionary<IntPtr, Border> borders = new Dictionary<IntPtr, Border>();
        private readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();

        internal BorderSession(Process process)
        {
            target = process;
            timer.Interval = 100;
            timer.Tick += Update;
            timer.Start();
        }

        private void Update(object sender, EventArgs arguments)
        {
            try
            {
                if (target.HasExited) { ExitThread(); return; }
                var marked = new List<IntPtr>();
                // Collect first: creating overlay windows inside EnumWindows is unsafe.
                Native.EnumWindows(delegate(IntPtr window, IntPtr parameter)
                {
                    if (Native.IsMarked(window, target.Id)) marked.Add(window);
                    return true;
                }, IntPtr.Zero);

                foreach (var window in new List<IntPtr>(borders.Keys))
                {
                    if (!marked.Contains(window)) { borders[window].Dispose(); borders.Remove(window); }
                }
                foreach (var window in marked)
                {
                    Border border;
                    if (!borders.TryGetValue(window, out border))
                    {
                        border = new Border();
                        borders.Add(window, border);
                    }
                    border.Follow(window);
                }
            }
            catch (Exception exception)
            {
                Console.Error.WriteLine(exception);
                ExitThread();
            }
        }

        protected override void ExitThreadCore()
        {
            timer.Stop();
            foreach (var border in borders.Values) border.Dispose();
            borders.Clear();
            base.ExitThreadCore();
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing) timer.Dispose();
            base.Dispose(disposing);
        }
    }

    public static class Runner
    {
        public static void Run(int processId)
        {
            bool created;
            using (var mutex = new Mutex(true, "Local\\AuroraAutomationBorder-" + processId, out created))
            {
                if (!created) return;
                try
                {
                    using (var process = Process.GetProcessById(processId))
                    {
                        // Device coordinates follow the viewer across monitor DPI changes.
                        Native.SetThreadDpiAwarenessContext(new IntPtr(-4));
                        Application.EnableVisualStyles();
                        using (var session = new BorderSession(process)) Application.Run(session);
                    }
                }
                catch (ArgumentException) { /* Target already exited before startup. */ }
                finally { mutex.ReleaseMutex(); }
            }
        }
    }
}
'@

[AuroraAutomation.Runner]::Run($ViewerProcessId)
