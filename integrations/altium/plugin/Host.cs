using DXP;
using System;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Windows.Forms;

[assembly: AssemblyVersion("0.9.2.0")]
[assembly: AssemblyFileVersion("0.9.2.0")]
[assembly: AssemblyTitle("JLC Export Workstation - AD")]
[assembly: AssemblyCompany("LZJ-I")]
[assembly: ComVisible(false)]

// Altium's managed extension loader looks for this exact namespace/factory.
namespace CSharpPlugin
{
    [ComVisible(true), ClassInterface(ClassInterfaceType.None)]
    public sealed class PluginFactory
    {
        public object InvokePluginFactory(IClient client) { return new Lceda.Altium.NativeModule(client); }
    }
}
namespace Lceda.Altium
{
    [ComVisible(true), ClassInterface(ClassInterfaceType.AutoDispatch)]
    public sealed class NativeModule : ServerModule
    {
        BridgeRuntime runtime;
        Timer timer;
        readonly string root;
        public NativeModule(IClient client) : base(client, "Lceda.Altium")
        {
            // Must match directories::ProjectDirs("", "LZJ-I", "lceda-assistant").
            root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "LZJ-I", "lceda-assistant", "config", "ad-bridge");
            try
            {
                runtime = new BridgeRuntime(root, new AdEditor(client));
                timer = new Timer { Interval = 400 };
                timer.Tick += delegate { runtime.Tick(); };
                timer.Start(); // No form, Run Script command or manual reconnect.
                Application.ApplicationExit += delegate { Shutdown(); };
                AppDomain.CurrentDomain.ProcessExit += delegate { Shutdown(); };
            }
            catch (Exception error)
            {
                Directory.CreateDirectory(root);
                File.WriteAllText(Path.Combine(root, "native-error.txt"), error.ToString());
            }
        }
        protected override IServerDocument NewDocumentInstance(string kind, string fileName) { return null; }
        protected override void InitializeCommands()
        {
            ((CommandLauncher)CommandLauncher).RegisterCommand("Connect", new CommandProc(Connect));
            ((CommandLauncher)CommandLauncher).RegisterCommand("Status", new CommandProc(Status));
        }
        void Connect(IServerDocumentView view, ref string parameters)
        {
            if (runtime != null) { runtime.Enable(); runtime.Tick(); }
            else MessageBox.Show("Plugin initialization failed. See " + Path.Combine(root, "native-error.txt"), "JLC Export Workstation");
        }
        void Status(IServerDocumentView view, ref string parameters)
        {
            MessageBox.Show(runtime == null ? "Connection unavailable. See native-error.txt in the workstation connection folder." :
                "Native plugin loaded (0.9.2). Synchronization status is shown in JLC Export Workstation.\n" + root, "JLC Export Workstation");
        }
        void Shutdown()
        {
            if (timer != null) { timer.Stop(); timer.Dispose(); timer = null; }
            if (runtime != null) { runtime.Dispose(); runtime = null; }
        }
    }
}
