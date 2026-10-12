// TEST ONLY. Not Altium's SDK, not shipped or used for a production build.
// Checks C# factory syntax and installer transactions without commercial DLLs.
namespace DXP
{
    public interface IClient { }
    public interface IServerDocument { }
    public interface IServerDocumentView { }
    public delegate void CommandProc(IServerDocumentView view, ref string parameters);
    public sealed class CommandLauncher { public void RegisterCommand(string name, CommandProc proc) { } }
    public abstract class ServerModule
    {
        protected object CommandLauncher = new CommandLauncher();
        protected ServerModule(IClient client, string name) { InitializeCommands(); }
        protected abstract IServerDocument NewDocumentInstance(string kind, string fileName);
        protected abstract void InitializeCommands();
    }
}
