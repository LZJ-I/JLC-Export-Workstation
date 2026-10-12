using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;

namespace Lceda.Altium
{
    // Interface invocation (rather than dynamic COM dispatch) is required for
    // Altium's managed SDK. Resolve from the installed version, fail explicitly
    // if an interface is absent, and never substitute guessed enum values.
    public sealed class Api
    {
        readonly Type[] types;
        public Api()
        {
            Assembly.Load("Altium.SDK.Interfaces");
            types = AppDomain.CurrentDomain.GetAssemblies().Where(a => a.GetName().Name.StartsWith("Altium.SDK", StringComparison.Ordinal))
                .SelectMany(a => a.GetTypes()).ToArray();
        }
        public Type Type(string name)
        {
            var type = types.SingleOrDefault(t => t.FullName == name);
            if (type == null) throw new NotSupportedException("Installed AD SDK lacks " + name);
            return type;
        }
        public Node Wrap(object value, string name)
        {
            if (value == null) throw new InvalidOperationException("AD returned null: " + name);
            var type = name == null ? null : Type(name);
            if (type != null && !type.IsInstanceOfType(value)) throw new NotSupportedException("AD object does not implement " + name);
            return new Node(this, value, type);
        }
        internal IEnumerable<MethodInfo> Methods(Node node, string names, int argc)
        {
            var candidates = node.Interface == null ? types.Where(t => t.IsInterface) : new[] { node.Interface }.Concat(node.Interface.GetInterfaces());
            var methods = candidates.SelectMany(t => t.GetMethods()).Where(m => names.Split('|').Contains(m.Name, StringComparer.OrdinalIgnoreCase) && m.GetParameters().Length == argc);
            // A typed node already passed QueryInterface in Wrap. Avoid doing a
            // COM interface query for every method on every footprint primitive.
            return node.Interface == null ? methods.Where(m => m.DeclaringType.IsInstanceOfType(node.Value)) : methods;
        }
        public void Need(string type, string names, int argc)
        {
            var t = Type(type);
            if (!new[] { t }.Concat(t.GetInterfaces()).SelectMany(i => i.GetMethods()).Any(m => names.Split('|').Contains(m.Name, StringComparer.OrdinalIgnoreCase) && m.GetParameters().Length == argc))
                throw new NotSupportedException("Installed AD SDK lacks " + type + "." + names);
        }
        public object Enum(string type, string value) { return System.Enum.Parse(Type(type), value, false); }
        internal static object ConvertArg(object value, System.Type type)
        {
            if (type.IsByRef) type = type.GetElementType();
            if (value is Node) value = ((Node)value).Value;
            if (value == null) return type.IsValueType ? Activator.CreateInstance(type) : null;
            if (type.IsInstanceOfType(value)) return value;
            if (type == typeof(IntPtr)) return new IntPtr(Convert.ToInt64(value, CultureInfo.InvariantCulture));
            if (type == typeof(UIntPtr)) return new UIntPtr(Convert.ToUInt64(value, CultureInfo.InvariantCulture));
            if (value is IntPtr) value = ((IntPtr)value).ToInt64();
            if (value is UIntPtr) value = ((UIntPtr)value).ToUInt64();
            if (type.IsEnum) return System.Enum.ToObject(type, Convert.ToInt64(value, CultureInfo.InvariantCulture));
            return Convert.ChangeType(value, type, CultureInfo.InvariantCulture);
        }
    }
    public sealed class Node
    {
        readonly Api api;
        public readonly object Value;
        public readonly Type Interface;
        public Node(Api owner, object value, Type type) { api = owner; Value = value; Interface = type; }
        public bool Has(string names, int argc) { return api.Methods(this, names, argc).Any(); }
        public object Call(string names, params object[] args)
        {
            foreach (string name in names.Split('|'))
            foreach (var method in api.Methods(this, name, args.Length))
            {
                object[] converted;
                try { converted = args.Select((a, i) => Api.ConvertArg(a, method.GetParameters()[i].ParameterType)).ToArray(); }
                catch (InvalidCastException) { continue; }
                catch (ArgumentException) { continue; }
                try { return method.Invoke(Value, converted); }
                catch (TargetInvocationException error) { throw new InvalidOperationException(name + ": " + error.InnerException.Message, error.InnerException); }
            }
            throw new NotSupportedException("Installed AD API lacks " + (Interface == null ? "object" : Interface.FullName) + "." + names + "(" + args.Length + ")");
        }
        public object Get(string name) { return Call("GetState_" + name + "|get_" + name + "|Get" + name + "|" + name); }
        public void Set(string name, object value) { Call("SetState_" + name + "|set_" + name + "|Set" + name, value); }
        public string Text(string name) { return Convert.ToString(Get(name), CultureInfo.InvariantCulture); }
        public long Number(string names) { return Convert.ToInt64(Call(names), CultureInfo.InvariantCulture); }
        public object Address { get { return Call("I_ObjectAddress|GetState_ObjectAddress|get_I_ObjectAddress"); } }
    }

    public sealed class AdEditor : ISyncEditor
    {
        readonly object rawClient;
        Api api;
        Node client, schServer, pcbServer;
        public AdEditor(object adClient) { rawClient = adClient; }
        Node Wrap(object value, string name) { return api.Wrap(value, name); }
        Node Open(string kind, string path)
        {
            object existing = client.Call("GetDocumentByPath", path);
            var doc = Wrap(existing ?? client.Call("OpenDocument", kind, path), "DXP.IServerDocument");
            if (!Fields.SamePath(doc.Text("FileName"), path)) throw new IOException("AD opened a different document than requested");
            client.Call("ShowDocument", doc);
            return doc;
        }
        Node Sch(string path)
        {
            Open("SchLib", path);
            var lib = Wrap(schServer.Call("GetSchDocumentByPath", path), "SCH.ISch_Lib");
            if (!lib.Get("ObjectId").Equals(api.Enum("SCH.TObjectId", "eSchLib"))) throw new IOException("Current document is not the requested SchLib");
            return lib;
        }
        Node Pcb(string path) { Open("PcbLib", path); return Wrap(pcbServer.Call("GetPCBLibraryByPath", path), "PCB.IPCB_Library"); }
        void RequireClean(string path)
        {
            object doc = client.Call("GetDocumentByPath", path);
            if (doc != null && Convert.ToBoolean(Wrap(doc, "DXP.IServerDocument").Get("Modified")))
                throw new IOException("Library has unsaved edits. Save or discard them in AD before syncing: " + path);
        }
        void CloseClean(string path)
        {
            RequireClean(path);
            object doc = client.Call("GetDocumentByPath", path);
            if (doc != null) client.Call("CloseDocument", doc);
            if (client.Call("GetDocumentByPath", path) != null) throw new IOException("AD did not close the clean library: " + path);
        }
        void Save(string kind, string path)
        {
            var doc = Wrap(client.Call("GetDocumentByPath", path), "DXP.IServerDocument");
            doc.Call("SetModified", true);
            object saved = doc.Call("DoFileSave", kind);
            if (saved is bool && !(bool)saved) throw new IOException("AD refused to save: " + path);
            RequireClean(path);
            if (!File.Exists(path)) throw new IOException("Saved file is missing: " + path);
            CloseClean(path); Open(kind, path);
        }
        Node Symbol(Node lib, string name)
        {
            if (name.Length == 0) return null;
            object comp = lib.Call("GetState_SchComponentByLibRef", name);
            return comp == null ? null : Wrap(comp, "SCH.ISch_Component");
        }
        Node Existing(Node lib, IDictionary<string, string> request)
        {
            var names = new List<string> { Fields.Text(request, "symbol"), Fields.Text(request, "legacy_symbol") };
            if (request.ContainsKey("existing_symbol")) names.Add(Fields.Text(request, "existing_symbol"));
            foreach (string name in names.Distinct(StringComparer.OrdinalIgnoreCase)) { var comp = Symbol(lib, name); if (comp != null) return comp; }
            return null;
        }
        Node Footprint(Node lib, string name)
        {
            object fp = lib.Call("GetComponentByName", name);
            return fp == null ? null : Wrap(fp, "PCB.IPCB_LibComponent");
        }
        List<Node> Primitives(Node footprint)
        {
            var result = new List<Node>(); var seen = new HashSet<string>();
            var iter = Wrap(footprint.Call("GroupIterator_Create"), "PCB.IPCB_GroupIterator");
            try
            {
                object next = iter.Call("FirstPCBObject");
                while (next != null)
                {
                    var primitive = Wrap(next, "PCB.IPCB_Primitive");
                    if (seen.Add(Convert.ToString(primitive.Address, CultureInfo.InvariantCulture))) result.Add(primitive);
                    if (result.Count > 200000) throw new IOException("Footprint primitive limit exceeded");
                    next = iter.Call("NextPCBObject");
                }
            }
            finally { footprint.Call("GroupIterator_Destroy", iter); }
            return result;
        }
        void SetLink(Node comp, string library, string name)
        {
            // Native source symbols deliberately have no implementation. Only
            // this NEW symbol receives a link; existing components are untouched.
            var impl = Wrap(comp.Call("AddSchImplementation"), "SCH.ISch_Implementation");
            impl.Set("ModelType", "PCBLIB"); impl.Set("ModelName", name);
            impl.Set("IsCurrent", true); impl.Set("UseComponentLibrary", false);
            impl.Call("ClearAllDatafileLinks"); impl.Call("AddDataFileLink", name, "", "PCBLib");
            if (Convert.ToInt32(impl.Get("DatafileLinkCount")) != 1) throw new IOException("AD did not create a footprint link");
            var link = Wrap(impl.Call("GetState_DatafileLink|GetDatafileLink|get_DatafileLink", 0), null);
            link.Set("Location", library);
        }
        void AttachStep(Node lib, Node fp, IDictionary<string, string> request)
        {
            string path = Fields.Text(request, "step");
            if (!File.Exists(path)) return;
            var body = Wrap(pcbServer.Call("PCBObjectFactory", api.Enum("PCB.TObjectId", "eComponentBodyObject"), api.Enum("PCB.TDimensionKind", "eNoDimension"), api.Enum("PCB.TObjectCreationMode", "eCreate_Default")), "PCB.IPCB_ComponentBody");
            var model = Wrap(body.Call("ModelFactory_FromFilename", path, false), "PCB.IPCB_Model");
            Func<string, int> number = key => Int32.Parse(Fields.Get(request, key), CultureInfo.InvariantCulture);
            model.Call("SetState", number("rx") / 1000.0, number("ry") / 1000.0, number("rz") / 1000.0, number("z"));
            if (body.Has("SetState_FromModel", 0)) body.Call("SetState_FromModel");
            body.Call("SetModel|SetState_Model", model);
            body.Call("MoveByXY", number("x") + fp.Number("GetState_XLocation|GetState_X"), number("y") + fp.Number("GetState_YLocation|GetState_Y"));
            fp.Call("AddPCBObject", body);
            var board = Wrap(lib.Get("Board"), "PCB.IPCB_Board"); board.Call("AddPCBObject", body);
            // Official API message constants: PCBM_BoardRegisteration = 2.
            pcbServer.Call("SendMessageToRobots", fp.Address, null, 2, body.Address);
            pcbServer.Call("SendMessageToRobots", board.Address, null, 2, body.Address);
        }
        void CheckCapabilities(bool symbols, bool footprints, bool model)
        {
            // Check signatures before creating target files or modifying objects.
            foreach (var spec in new[] { "DXP.IClient|OpenDocument|2", "DXP.IClient|ShowDocument|1", "DXP.IClient|CloseDocument|1", "DXP.IClient|GetDocumentByPath|1", "DXP.IServerDocument|GetFileName|0", "DXP.IServerDocument|GetModified|GetState_Modified|get_Modified|0", "DXP.IServerDocument|SetModified|1", "DXP.IServerDocument|DoFileSave|1" }) Need(spec);
            if (symbols)
                foreach (var spec in new[] { "SCH.ISch_Lib|GetState_SchComponentByLibRef|1", "SCH.ISch_Lib|AddSchComponent|1", "SCH.ISch_Lib|SetState_Current_SchComponent|SetState_CurrentSchComponent|1", "SCH.ISch_Lib|GraphicallyInvalidate|0", "SCH.ISch_Component|Replicate|0", "SCH.ISch_Component|SetState_LibReference|1", "SCH.ISch_Component|SetState_DesignItemId|1", "SCH.ISch_Component|I_ObjectAddress|0", "SCH.ISch_ProcessControl|PreProcess|2", "SCH.ISch_ProcessControl|PostProcess|2", "SCH.ISch_RobotManager|SendMessage|4" }) Need(spec);
            if (footprints)
                foreach (var spec in new[] { "PCB.IPCB_Library|CreateNewComponent|0", "PCB.IPCB_Library|RegisterComponent|1", "PCB.IPCB_Library|GetComponentByName|1", "PCB.IPCB_LibComponent|CopyTo|2", "PCB.IPCB_LibComponent|SetState_Name|SetState_Pattern|1", "PCB.IPCB_LibComponent|GroupIterator_Create|0", "PCB.IPCB_LibComponent|GroupIterator_Destroy|1", "PCB.IPCB_Primitive|I_ObjectAddress|0", "PCB.IPCB_Primitive|MoveByXY|2", "PCB.IPCB_GroupIterator|FirstPCBObject|0", "PCB.IPCB_GroupIterator|NextPCBObject|0", "PCB.IPCB_ServerInterface|SendMessageToRobots|4", "PCB.IPCB_ServerInterface|PreProcess|0", "PCB.IPCB_ServerInterface|PostProcess|0" }) Need(spec);
            if (symbols && footprints)
                foreach (var spec in new[] { "SCH.ISch_Component|AddSchImplementation|0", "SCH.ISch_Implementation|SetState_ModelType|1", "SCH.ISch_Implementation|SetState_ModelName|1", "SCH.ISch_Implementation|SetState_IsCurrent|1", "SCH.ISch_Implementation|SetState_UseComponentLibrary|1", "SCH.ISch_Implementation|ClearAllDatafileLinks|0", "SCH.ISch_Implementation|AddDataFileLink|3", "SCH.ISch_Implementation|GetState_DatafileLink|GetDatafileLink|get_DatafileLink|1" }) Need(spec);
            if (model)
                foreach (var spec in new[] { "PCB.IPCB_ComponentBody|ModelFactory_FromFilename|2", "PCB.IPCB_ComponentBody|SetModel|SetState_Model|1", "PCB.IPCB_Model|SetState|4" }) Need(spec);
            if (footprints) { api.Enum("PCB.TCopyMode", "eFullCopy"); api.Enum("PCB.TObjectId", "eComponentBodyObject"); }
            if (symbols) api.Need("SCH.ISch_ServerInterface", "GetSchDocumentByPath", 1);
            if (footprints) api.Need("PCB.IPCB_ServerInterface", "GetPCBLibraryByPath", 1);
        }
        void Need(string spec)
        {
            string[] parts = spec.Split('|');
            api.Need(parts[0], String.Join("|", parts.Skip(1).Take(parts.Length - 2)), Int32.Parse(parts.Last(), CultureInfo.InvariantCulture));
        }
        void Prepare(string path, string seed, string backup)
        {
            CloseClean(path);
            if (File.Exists(path)) File.Copy(path, backup, false);
            else { Directory.CreateDirectory(Path.GetDirectoryName(path)); File.Copy(seed, path, false); }
        }
        public void Execute(Dictionary<string, string> request, Dictionary<string, string> reply, Action<string> phase)
        {
            if (api == null) { api = new Api(); client = Wrap(rawClient, "DXP.IClient"); }
            string schPath = Fields.Text(request, "target_sch"), pcbPath = Fields.Text(request, "target_pcb"), job = Fields.Text(request, "job");
            string name = Fields.Text(request, "symbol"), fpName = Fields.Text(request, "footprint"), stage = "preflight";
            bool addSch = false, addPcb = false, createSch = !File.Exists(schPath), createPcb = !File.Exists(pcbPath);
            Node sourceSch = null, sourceFp = null, targetSch = null, targetPcb = null;
            try
            {
                if (name.Length == 0 && fpName.Length == 0) throw new IOException("No symbol or footprint requested");
                phase(stage); CheckCapabilities(name.Length > 0, fpName.Length > 0, fpName.Length > 0 && File.Exists(Fields.Text(request, "step")));
                if (name.Length > 0) RequireClean(schPath);
                if (fpName.Length > 0) RequireClean(pcbPath);
                if (name.Length > 0)
                {
                    client.Call("StartServer", "SCH"); schServer = Wrap(client.Call("GetServerModuleByName", "SCH"), "SCH.ISch_ServerInterface");
                    var source = Sch(Fields.Text(request, "source_sch")); sourceSch = Symbol(source, name);
                    if (sourceSch == null) throw new IOException("Source symbol is missing");
                }
                if (fpName.Length > 0)
                {
                    client.Call("StartServer", "PCB"); pcbServer = Wrap(client.Call("GetServerModuleByName", "PCB"), "PCB.IPCB_ServerInterface");
                    sourceFp = Footprint(Pcb(Fields.Text(request, "source_pcb")), fpName);
                    if (sourceFp == null) throw new IOException("Source footprint is missing");
                }
                int expectedPrimitives = sourceFp == null ? 0 : Primitives(sourceFp).Count;
                // Resolve the new symbol's implementation API on a detached
                // clone before any target is created or edited.
                Node newSch = null;
                if (sourceSch != null)
                {
                    Open("SchLib", Fields.Text(request, "source_sch"));
                    newSch = Wrap(sourceSch.Call("Replicate"), "SCH.ISch_Component");
                    if (fpName.Length > 0) SetLink(newSch, pcbPath, fpName);
                }
                if (name.Length > 0) { Prepare(schPath, Fields.Text(request, "seed_sch"), Path.Combine(job, "before.SchLib")); targetSch = Sch(schPath); }
                if (fpName.Length > 0) { Prepare(pcbPath, Fields.Text(request, "seed_pcb"), Path.Combine(job, "before.PcbLib")); targetPcb = Pcb(pcbPath); }
                if (Int64.Parse(request["expires"], CultureInfo.InvariantCulture) < Fields.UnixNow) throw new IOException("Request expired before editing");
                var existingSch = targetSch == null ? null : Existing(targetSch, request);
                var existingFp = targetPcb == null ? null : Footprint(targetPcb, fpName);
                if (existingSch != null) newSch = null;
                if (targetPcb != null && existingFp == null)
                {
                    stage = "copy_footprint"; phase(stage); Open("PcbLib", pcbPath); pcbServer.Call("PreProcess");
                    try
                    {
                        var fp = Wrap(targetPcb.Call("CreateNewComponent"), "PCB.IPCB_LibComponent"); addPcb = true;
                        sourceFp.Call("CopyTo", fp, api.Enum("PCB.TCopyMode", "eFullCopy"));
                        long dx = fp.Number("GetState_XLocation|GetState_X") - sourceFp.Number("GetState_XLocation|GetState_X");
                        long dy = fp.Number("GetState_YLocation|GetState_Y") - sourceFp.Number("GetState_YLocation|GetState_Y");
                        var primitives = Primitives(fp);
                        if (primitives.Count != expectedPrimitives) throw new IOException("Copied footprint primitive count differs from source");
                        if (dx != 0 || dy != 0) foreach (var primitive in primitives) primitive.Call("MoveByXY", dx, dy);
                        fp.Call("SetState_Pattern|SetState_Name", fpName); targetPcb.Call("RegisterComponent", fp);
                        AttachStep(targetPcb, fp, request); targetPcb.Set("CurrentComponent", fp);
                        Wrap(targetPcb.Get("Board"), "PCB.IPCB_Board").Call("ViewManager_FullUpdate");
                    }
                    finally { pcbServer.Call("PostProcess"); }
                }
                if (newSch != null)
                {
                    stage = "copy_symbol"; phase(stage); Open("SchLib", schPath);
                    var control = Wrap(schServer.Get("ProcessControl"), "SCH.ISch_ProcessControl"); control.Call("PreProcess", targetSch, "");
                    try
                    {
                        targetSch.Call("AddSchComponent", newSch); addSch = true;
                        newSch.Set("LibReference", name); newSch.Set("DesignItemId", name);
                        // Official API message constant: SCHM_PrimitiveRegistration = 1.
                        Wrap(schServer.Get("RobotManager"), "SCH.ISch_RobotManager").Call("SendMessage", null, null, 1, newSch.Address);
                        targetSch.Call("SetState_Current_SchComponent|SetState_CurrentSchComponent", newSch); targetSch.Call("GraphicallyInvalidate");
                    }
                    finally { control.Call("PostProcess", targetSch, "Edit"); }
                }
                stage = "saving"; phase(stage);
                // No design-object references are reused after closing a target.
                targetSch = null; targetPcb = null; newSch = null; existingSch = null; existingFp = null;
                if (fpName.Length > 0 && (addPcb || createPcb)) Save("PcbLib", pcbPath);
                if (name.Length > 0 && (addSch || createSch)) Save("SchLib", schPath);
                stage = "verifying"; phase(stage); string actual = "";
                if (name.Length > 0)
                {
                    var comp = Existing(Sch(schPath), request);
                    if (comp == null) throw new IOException("AD cannot read the saved symbol back");
                    actual = comp.Text("LibReference");
                }
                if (fpName.Length > 0)
                {
                    var fp = Footprint(Pcb(pcbPath), fpName);
                    if (fp == null) throw new IOException("AD cannot read the saved footprint back");
                    if (addPcb && File.Exists(Fields.Text(request, "step")))
                    {
                        bool hasModel = Primitives(fp).Any(p => p.Get("ObjectId").Equals(api.Enum("PCB.TObjectId", "eComponentBodyObject")) && Wrap(p.Value, "PCB.IPCB_ComponentBody").Get("Model") != null);
                        if (!hasModel) throw new IOException("Saved footprint has no readable 3D model");
                        reply["model_verified"] = "1";
                    }
                }
                reply["component"] = Fields.Hex(actual); reply["footprint"] = Fields.Hex(fpName);
                reply["existing"] = addSch ? "0" : "1"; reply["footprint_existing"] = addPcb ? "0" : "1";
                reply["verified"] = "1"; reply["ok"] = "1"; phase("complete");
            }
            catch (Exception error)
            {
                // Keep partial saved results, unsaved AD edits and original backups.
                // Never restore a file behind the editor's in-memory document.
                throw new IOException(stage + ": " + error.Message + ". Pre-sync backups: " + job, error);
            }
            finally
            {
                foreach (string key in new[] { "source_sch", "source_pcb" })
                {
                    try { CloseClean(Fields.Text(request, key)); } catch { }
                }
            }
        }
    }
}
