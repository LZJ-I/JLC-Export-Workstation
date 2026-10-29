// Native host runtime. C# 5 / .NET Framework; no Altium binaries are distributed.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;

namespace Lceda.Altium
{
    public interface ISyncEditor
    {
        void Execute(Dictionary<string, string> request, Dictionary<string, string> reply, Action<string> phase);
    }

    public static class Fields
    {
        public static Dictionary<string, string> Read(string path)
        {
            if (new FileInfo(path).Length > 128 * 1024) throw new IOException("IPC record exceeds 128 KiB");
            var fields = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (string line in File.ReadAllLines(path, Encoding.UTF8))
            {
                if (line.Length == 0) continue;
                int split = line.IndexOf('=');
                if (split < 1 || fields.ContainsKey(line.Substring(0, split))) throw new IOException("Invalid or duplicate IPC field");
                fields.Add(line.Substring(0, split), line.Substring(split + 1));
            }
            return fields;
        }
        public static void Write(string path, IDictionary<string, string> fields, bool durable = true)
        {
            string temp = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            try
            {
                using (var file = new FileStream(temp, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                using (var writer = new StreamWriter(file, new UTF8Encoding(false)))
                {
                    foreach (var field in fields.OrderBy(p => p.Key, StringComparer.Ordinal))
                    {
                        if (field.Value.IndexOfAny(new[] { '\r', '\n', '\0' }) >= 0) throw new IOException("Invalid IPC value");
                        writer.WriteLine(field.Key + "=" + field.Value);
                    }
                    writer.Flush(); file.Flush(durable);
                }
                if (File.Exists(path)) File.Replace(temp, path, null);
                else File.Move(temp, path);
            }
            finally { if (File.Exists(temp)) File.Delete(temp); }
        }
        public static string Get(IDictionary<string, string> fields, string key)
        {
            string value;
            if (!fields.TryGetValue(key, out value)) throw new IOException("Missing IPC field: " + key);
            return value;
        }
        public static string Hex(string text)
        {
            return String.Concat(text.Select(c => ((int)c).ToString("X4", CultureInfo.InvariantCulture)));
        }
        public static string Text(IDictionary<string, string> fields, string key)
        {
            string hex = Get(fields, key);
            if (hex.Length % 4 != 0) throw new IOException("Invalid UTF-16 hex: " + key);
            var bytes = new byte[hex.Length / 2];
            for (int i = 0; i < hex.Length; i += 4)
            {
                ushort unit = UInt16.Parse(hex.Substring(i, 4), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
                bytes[i / 2] = (byte)(unit & 255); bytes[i / 2 + 1] = (byte)(unit >> 8);
            }
            string text = new UnicodeEncoding(false, false, true).GetString(bytes);
            if (text.IndexOfAny(new[] { '\r', '\n', '\0' }) >= 0) throw new IOException("Invalid text: " + key);
            return text;
        }
        public static long UnixNow { get { return (long)(DateTime.UtcNow - new DateTime(1970, 1, 1)).TotalSeconds; } }
        public static bool Token(string value)
        {
            return value.Length > 0 && value.Length <= 128 && value.All(c => c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-' || c == '_');
        }
        public static bool SamePath(string a, string b)
        {
            return String.Equals(Path.GetFullPath(a).TrimEnd('\\'), Path.GetFullPath(b).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase);
        }
    }

    // Tick is called exclusively by a WinForms timer on Altium's UI thread.
    // Downloads and source generation stay in the workstation process.
    public sealed class BridgeRuntime : IDisposable
    {
        readonly string root;
        readonly ISyncEditor editor;
        readonly int uiThread;
        readonly FileStream owner;
        readonly string session = Guid.NewGuid().ToString("N");
        bool inTick, disposed;
        public string Session { get { return session; } }
        public BridgeRuntime(string directory, ISyncEditor syncEditor)
        {
            root = Path.GetFullPath(directory); editor = syncEditor;
            uiThread = Thread.CurrentThread.ManagedThreadId;
            Directory.CreateDirectory(root);
            string heartbeat = Path.Combine(root, "heartbeat.ini");
            if (File.Exists(heartbeat))
            {
                var prior = Fields.Read(heartbeat); string state;
                if (prior.TryGetValue("state", out state) &&
                    (state == "ready" || state == "busy") &&
                    (DateTime.UtcNow - File.GetLastWriteTimeUtc(heartbeat)).TotalSeconds < (state == "busy" ? 130 : 4))
                    throw new IOException("Another AD connection is active. Close the old script connection or the other AD instance.");
            }
            owner = new FileStream(Path.Combine(root, "native-owner.lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
        }
        void Heartbeat(string state, string error)
        {
            var fields = new Dictionary<string, string> {
                { "protocol", "1" }, { "session", session }, { "state", state },
                { "backend", "native" }, { "version", "0.9.3" }, { "pid", System.Diagnostics.Process.GetCurrentProcess().Id.ToString() }
            };
            if (error != null) fields["error"] = Fields.Hex(error);
            Fields.Write(Path.Combine(root, "heartbeat.ini"), fields, false);
        }
        public void Enable()
        {
            File.WriteAllText(Path.Combine(root, "enabled"), "1");
            if (File.Exists(Path.Combine(root, "stop"))) File.Delete(Path.Combine(root, "stop"));
        }
        public void Tick()
        {
            if (disposed || inTick) return;
            if (Thread.CurrentThread.ManagedThreadId != uiThread) throw new InvalidOperationException("AD edits must run on the owning UI thread");
            inTick = true;
            try
            {
                if (!File.Exists(Path.Combine(root, "enabled")) || File.Exists(Path.Combine(root, "stop"))) { Heartbeat("stopped", null); return; }
                Heartbeat("ready", null);
                string pending = Path.Combine(root, "pending.ini");
                if (!File.Exists(pending)) return;
                var request = Fields.Read(pending);
                // Retired sessions and expired requests are never replayed.
                if (Fields.Get(request, "session") != session) { File.Delete(pending); return; }
                string job = Validate(request);
                var persisted = Fields.Read(Path.Combine(job, "request.ini"));
                if (request.Count != persisted.Count || request.Any(p => !persisted.ContainsKey(p.Key) || persisted[p.Key] != p.Value))
                    throw new IOException("Pending pointer differs from the immutable job request");
                File.Move(pending, Path.Combine(job, "claimed.ini"));
                var reply = new Dictionary<string, string> {
                    { "protocol", "1" }, { "session", session }, { "request", request["request"] }, { "ok", "0" }, { "verified", "0" }
                };
                try
                {
                    Heartbeat("busy", null);
                    Action<string> phase = name => Fields.Write(Path.Combine(job, "status.ini"), new Dictionary<string, string> {
                        { "protocol", "1" }, { "session", session }, { "request", request["request"] }, { "phase", name }
                    });
                    phase("opening");
                    if (Int64.Parse(request["expires"], CultureInfo.InvariantCulture) < Fields.UnixNow) throw new IOException("Request expired before editing");
                    editor.Execute(request, reply, phase);
                    if (reply["ok"] != "1" || reply["verified"] != "1") throw new IOException("Editor did not confirm saved readback");
                }
                catch (Exception error) { reply["ok"] = "0"; reply["verified"] = "0"; reply["error"] = Fields.Hex(error.Message); }
                Fields.Write(Path.Combine(job, "reply.ini"), reply);
                Heartbeat("ready", null);
            }
            catch (Exception error)
            {
                // Never leak an exception into the Altium message loop.
                try { Heartbeat("error", error.Message); File.WriteAllText(Path.Combine(root, "native-error.txt"), error.ToString()); } catch { }
            }
            finally { inTick = false; }
        }
        string Validate(Dictionary<string, string> request)
        {
            if (Fields.Get(request, "protocol") != "1" || !Fields.Token(Fields.Get(request, "request"))) throw new IOException("Unsupported bridge request");
            string job = Fields.Text(request, "job");
            if (!Fields.SamePath(job, Path.Combine(root, "jobs", request["request"]))) throw new IOException("Job is outside the private bridge folder");
            foreach (var pair in new[] { "source_sch|source.SchLib", "source_pcb|source.PcbLib", "seed_sch|empty.SchLib", "seed_pcb|empty.PcbLib", "step|model.step" })
            {
                string[] parts = pair.Split('|');
                if (!Fields.SamePath(Fields.Text(request, parts[0]), Path.Combine(job, parts[1]))) throw new IOException("Source is outside its job folder");
            }
            string sch = Fields.Text(request, "target_sch"), pcb = Fields.Text(request, "target_pcb");
            if (!Path.IsPathRooted(sch) || !Path.IsPathRooted(pcb) || !String.Equals(Path.GetExtension(sch), ".SchLib", StringComparison.OrdinalIgnoreCase) ||
                !Fields.SamePath(Path.ChangeExtension(sch, ".PcbLib"), pcb)) throw new IOException("Invalid paired target library paths");
            long expiry = Int64.Parse(Fields.Get(request, "expires"), CultureInfo.InvariantCulture);
            if (expiry > Fields.UnixNow + 125) throw new IOException("Request deadline is too far in the future");
            return job;
        }
        public void Dispose()
        {
            if (disposed) return; disposed = true;
            try { Heartbeat("stopped", null); } catch { }
            owner.Dispose();
        }
    }
}
