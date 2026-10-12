using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading;
using Lceda.Altium;

// Tests the real native IPC runtime, never pretends to be Altium's editor.
sealed class RecordingEditor : ISyncEditor
{
    public int Calls;
    public bool Fail;
    public void Execute(Dictionary<string, string> request, Dictionary<string, string> reply, Action<string> phase)
    {
        Calls++; phase("preflight");
        if (Fail) throw new IOException("unsaved library");
        phase("saving"); phase("verifying");
        reply["ok"] = "1"; reply["verified"] = "1";
        reply["component"] = Fields.Hex("10k_电阻_C25804"); reply["footprint"] = Fields.Hex("C25804");
        reply["existing"] = "1"; reply["footprint_existing"] = "1";
    }
}
static class RuntimeTests
{
    static int count;
    static void Check(bool condition, string test) { if (!condition) throw new Exception(test); count++; Console.WriteLine("PASS " + test); }
    static Dictionary<string, string> Request(string root, string session, string id)
    {
        string job = Path.Combine(root, "jobs", id); Directory.CreateDirectory(job);
        var fields = new Dictionary<string, string> { { "protocol", "1" }, { "session", session }, { "request", id }, { "expires", (Fields.UnixNow + 120).ToString(CultureInfo.InvariantCulture) }, { "job", Fields.Hex(job) }, { "target_sch", Fields.Hex(Path.Combine(root, "bound", "用户库.SchLib")) }, { "target_pcb", Fields.Hex(Path.Combine(root, "bound", "用户库.PcbLib")) } };
        foreach (string pair in new[] { "source_sch|source.SchLib", "source_pcb|source.PcbLib", "seed_sch|empty.SchLib", "seed_pcb|empty.PcbLib", "step|model.step" }) { var p = pair.Split('|'); fields[p[0]] = Fields.Hex(Path.Combine(job, p[1])); }
        return fields;
    }
    static void Submit(string root, Dictionary<string, string> request)
    {
        Fields.Write(Path.Combine(Fields.Text(request, "job"), "request.ini"), request);
        Fields.Write(Path.Combine(root, "pending.ini"), request);
    }
    public static int Main(string[] args)
    {
        string root = Path.Combine(args[0], "runtime-用户-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root); File.WriteAllText(Path.Combine(root, "enabled"), "1");
        try
        {
            var editor = new RecordingEditor(); string oldSession;
            using (var runtime = new BridgeRuntime(root, editor))
            {
                oldSession = runtime.Session; runtime.Tick();
                var heartbeat = Fields.Read(Path.Combine(root, "heartbeat.ini"));
                Check(heartbeat["state"] == "ready" && heartbeat["backend"] == "native", "native heartbeat without a form");
                bool refused = false; try { using (var duplicate = new BridgeRuntime(root, editor)) { } } catch (IOException) { refused = true; }
                Check(refused, "second editor cannot steal an active connection");
                var request = Request(root, runtime.Session, "normal"); Submit(root, request); runtime.Tick();
                var reply = Fields.Read(Path.Combine(Fields.Text(request, "job"), "reply.ini"));
                Check(reply["session"] == runtime.Session && reply["request"] == "normal" && reply["verified"] == "1" && editor.Calls == 1 && !File.Exists(Path.Combine(root, "pending.ini")), "request is claimed once and reply belongs to its session");
                Check(Fields.Text(reply, "component") == "10k_电阻_C25804", "Unicode paths and result text round trip");
                runtime.Tick(); Check(editor.Calls == 1, "completed request is not replayed");
                request = Request(root, runtime.Session, "expired"); request["expires"] = (Fields.UnixNow - 1).ToString(); Submit(root, request); runtime.Tick();
                reply = Fields.Read(Path.Combine(Fields.Text(request, "job"), "reply.ini"));
                Check(reply["ok"] == "0" && editor.Calls == 1, "expired request cannot start an edit");
                request = Request(root, runtime.Session, "failure"); Submit(root, request); editor.Fail = true; runtime.Tick();
                reply = Fields.Read(Path.Combine(Fields.Text(request, "job"), "reply.ini"));
                Check(reply["verified"] == "0" && Fields.Text(reply, "error").Contains("unsaved"), "editor failure is reported without success or replay");
                editor.Fail = false;
                request = Request(root, runtime.Session, "tamper"); Submit(root, request); request["target_sch"] = Fields.Hex(Path.Combine(root, "other.SchLib")); Fields.Write(Path.Combine(root, "pending.ini"), request); runtime.Tick();
                Check(Fields.Read(Path.Combine(root, "heartbeat.ini"))["state"] == "error" && editor.Calls == 2, "modified pending pointer cannot redirect a saved request");
                File.Delete(Path.Combine(root, "pending.ini"));
                request = Request(root, runtime.Session, "escape"); request["job"] = Fields.Hex(Path.Combine(root, "..", "outside")); Fields.Write(Path.Combine(root, "pending.ini"), request); runtime.Tick();
                Check(Fields.Read(Path.Combine(root, "heartbeat.ini"))["state"] == "error" && editor.Calls == 2, "job directory traversal is rejected"); File.Delete(Path.Combine(root, "pending.ini"));
                Exception wrongThread = null; var thread = new Thread(() => { try { runtime.Tick(); } catch (Exception e) { wrongThread = e; } }); thread.Start(); thread.Join();
                Check(wrongThread is InvalidOperationException && editor.Calls == 2, "non-UI thread cannot edit AD");
                File.WriteAllText(Path.Combine(root, "stop"), "stop"); runtime.Tick();
                Check(Fields.Read(Path.Combine(root, "heartbeat.ini"))["state"] == "stopped", "file mode stops the native connection");
                runtime.Enable(); runtime.Tick(); Check(Fields.Read(Path.Combine(root, "heartbeat.ini"))["state"] == "ready", "connection can resume without restarting AD");
                request = Request(root, runtime.Session, "retired"); Submit(root, request);
            }
            using (var runtime = new BridgeRuntime(root, editor))
            {
                runtime.Tick(); Check(runtime.Session != oldSession && editor.Calls == 2 && !File.Exists(Path.Combine(root, "pending.ini")), "restart never replays an old session's request");
            }
            string duplicateFields = Path.Combine(root, "duplicate.ini"); File.WriteAllText(duplicateFields, "session=a\nsession=b\n");
            bool duplicateRejected = false; try { Fields.Read(duplicateFields); } catch (IOException) { duplicateRejected = true; }
            Check(duplicateRejected, "duplicate protocol keys rejected");
            bool unicodeRejected = false; try { Fields.Text(new Dictionary<string, string> { { "x", "D800" } }, "x"); } catch (System.Text.DecoderFallbackException) { unicodeRejected = true; }
            Check(unicodeRejected, "malformed Unicode rejected");
            Console.WriteLine("Native runtime: " + count + " checks passed (no AD editor)."); return 0;
        }
        finally { Directory.Delete(root, true); }
    }
}
