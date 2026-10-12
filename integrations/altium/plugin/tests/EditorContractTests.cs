using System;
using System.Collections.Generic;
using System.IO;
using Lceda.Altium;

sealed class UnsupportedClient : DXP.IClient { }
static class EditorContractTests
{
    public static int Main(string[] args)
    {
        string root = Path.Combine(args[0], "unsupported-sdk"); Directory.CreateDirectory(root);
        string target = Path.Combine(root, "Original.SchLib"); File.WriteAllText(target, "original library bytes");
        var request = new Dictionary<string, string> { { "target_sch", Fields.Hex(target) }, { "target_pcb", Fields.Hex(Path.ChangeExtension(target, ".PcbLib")) }, { "job", Fields.Hex(root) }, { "symbol", Fields.Hex("C25804") }, { "footprint", "" } };
        try
        {
            new AdEditor(new UnsupportedClient()).Execute(request, new Dictionary<string, string>(), delegate { });
            throw new Exception("An unsupported SDK was accepted");
        }
        catch (IOException error)
        {
            if (!error.Message.Contains("Installed AD SDK lacks") || File.ReadAllText(target) != "original library bytes" || Directory.GetFiles(root).Length != 1) throw;
        }
        Console.WriteLine("PASS incompatible editor interfaces rejected before creating or modifying target libraries (test contract only)");
        return 0;
    }
}
