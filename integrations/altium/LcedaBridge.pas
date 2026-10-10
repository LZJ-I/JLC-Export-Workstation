{ JLC Export Workstation: local AD editor bridge, protocol 1.
  Independent implementation of the open-library / append / save workflow.
  API references and validation limitations: README_ZH.md. }

Var
    IpcRoot, SessionId, LastFailure : String;
    InTick, BridgeRunning : Boolean;

Procedure Fail(Message : String);
Begin
    LastFailure := Message;
    Raise(Message);
End;

Function DecodeText(Value : String) : String;
Var I : Integer;
Begin
    Result := '';
    If (Length(Value) Mod 4) <> 0 Then
        Fail('Invalid UTF-16 wire value');
    I := 1;
    While I <= Length(Value) Do
    Begin
        Result := Result + Chr(StrToInt('$' + Copy(Value, I, 4)));
        I := I + 4;
    End;
End;

Function EncodeText(Value : String) : String;
Var I : Integer;
Begin
    Result := '';
    For I := 1 To Length(Value) Do
        Result := Result + IntToHex(Ord(Value[I]), 4);
End;

Procedure WriteFields(Path : String; Fields : TStringList);
Var Temp : String;
Begin
    Temp := Path + '.writing';
    Fields.SaveToFile(Temp);
    If FileExists(Path) Then
        If Not DeleteFile(Path) Then Fail('Cannot replace IPC file: ' + Path);
    If Not RenameFile(Temp, Path) Then Fail('Cannot publish IPC file: ' + Path);
End;

Procedure Heartbeat(State : String);
Var Fields : TStringList;
Begin
    Fields := TStringList.Create;
    Try
        Fields.Values['protocol'] := '1';
        Fields.Values['session'] := SessionId;
        Fields.Values['state'] := State;
        WriteFields(IpcRoot + '\heartbeat.ini', Fields);
    Finally
        Fields.Free;
    End;
End;

Procedure Phase(Request : TStringList; Value : String);
Var Fields : TStringList;
Begin
    Fields := TStringList.Create;
    Try
        Fields.Values['protocol'] := '1';
        Fields.Values['session'] := SessionId;
        Fields.Values['request'] := Request.Values['request'];
        Fields.Values['phase'] := Value;
        WriteFields(DecodeText(Request.Values['job']) + '\status.ini', Fields);
        StatusLabel.Caption := Value + ': ' + DecodeText(Request.Values['symbol']);
    Finally
        Fields.Free;
    End;
End;

Procedure CopyFileBytes(Source, Target : String);
Var Input, Output : TFileStream;
Begin
    Input := TFileStream.Create(Source, fmOpenRead Or fmShareDenyWrite);
    Try
        Output := TFileStream.Create(Target, fmCreate);
        Try Output.CopyFrom(Input, 0);
        Finally Output.Free; End;
    Finally Input.Free; End;
End;

Procedure RequireClean(Path : String);
Var Doc : IServerDocument;
Begin
    Doc := Client.GetDocumentByPath(Path);
    If Doc <> Nil Then
        If Doc.Modified Then
            Fail('Save your unsaved library edits in AD first: ' + Path);
End;

Function OpenLibrary(Kind, Path : String) : IServerDocument;
Begin
    Result := Client.GetDocumentByPath(Path);
    If Result = Nil Then Result := Client.OpenDocument(Kind, Path);
    If Result = Nil Then Fail('AD could not open ' + Path);
    Client.ShowDocument(Result);
End;

Procedure CloseCleanLibrary(Path : String);
Var Doc : IServerDocument;
Begin
    RequireClean(Path);
    Doc := Client.GetDocumentByPath(Path);
    If Doc <> Nil Then
    Begin
        Client.CloseDocument(Doc);
        Doc := Nil;
        If Client.GetDocumentByPath(Path) <> Nil Then
            Fail('AD could not close the clean library for refresh: ' + Path);
    End;
End;

Function OpenSch(Path : String) : ISch_Lib;
Var Doc : IServerDocument;
Begin
    Doc := OpenLibrary('SchLib', Path);
    Result := SchServer.GetSchDocumentByPath(Path);
    If Result = Nil Then Fail('Target SchLib is not loaded: ' + Path);
    If Result.ObjectId <> eSchLib Then Fail('Target is not a SchLib');
End;

Function OpenPcb(Path : String) : IPCB_Library;
Var Doc : IServerDocument;
Begin
    Doc := OpenLibrary('PcbLib', Path);
    Result := PCBServer.GetPCBLibraryByPath(Path);
    If Result = Nil Then Fail('Target PcbLib is not loaded: ' + Path);
End;

Function FindSymbol(Lib : ISch_Lib; Name : String) : ISch_Component;
Var Iter : ISch_Iterator; Comp : ISch_Component;
Begin
    Result := Nil;
    Iter := Lib.SchLibIterator_Create;
    If Iter = Nil Then Fail('Cannot enumerate schematic library');
    Try
        Iter.AddFilter_ObjectSet(MkSet(eSchComponent));
        Comp := Iter.FirstSchObject;
        While Comp <> Nil Do
        Begin
            If UpperCase(Comp.LibReference) = UpperCase(Name) Then
            Begin Result := Comp; Exit; End;
            Comp := Iter.NextSchObject;
        End;
    Finally Lib.SchIterator_Destroy(Iter); End;
End;

Function FindTargetSymbol(Lib : ISch_Lib; Name, SupplierId : String) : ISch_Component;
Var Iter : ISch_Iterator; Comp : ISch_Component; Suffix, Reference : String;
Begin
    Result := FindSymbol(Lib, Name);
    If Result <> Nil Then Exit;
    Result := FindSymbol(Lib, SupplierId);
    If Result <> Nil Then Exit;
    Suffix := '_' + UpperCase(SupplierId);
    Iter := Lib.SchLibIterator_Create;
    If Iter = Nil Then Fail('Cannot enumerate schematic library');
    Try
        Iter.AddFilter_ObjectSet(MkSet(eSchComponent));
        Comp := Iter.FirstSchObject;
        While Comp <> Nil Do
        Begin
            Reference := UpperCase(Comp.LibReference);
            If (Length(Reference) >= Length(Suffix)) And
               (Copy(Reference, Length(Reference) - Length(Suffix) + 1, Length(Suffix)) = Suffix) Then
            Begin
                If Result <> Nil Then Fail('More than one symbol uses supplier ID ' + SupplierId);
                Result := Comp;
            End;
            Comp := Iter.NextSchObject;
        End;
    Finally Lib.SchIterator_Destroy(Iter); End;
End;

Function PrimitiveCount(Footprint : IPCB_LibComponent) : Integer;
Var Iter : IPCB_GroupIterator; Prim : IPCB_Primitive; Seen : TStringList; Key : String;
Begin
    Result := 0;
    Seen := TStringList.Create;
    Iter := Footprint.GroupIterator_Create;
    If Iter = Nil Then Fail('Cannot enumerate footprint');
    Try
        Prim := Iter.FirstPCBObject;
        While Prim <> Nil Do
        Begin
            Key := IntToStr(Prim.I_ObjectAddress);
            If Seen.IndexOf(Key) < 0 Then
            Begin Seen.Add(Key); Result := Result + 1; End;
            Prim := Iter.NextPCBObject;
        End;
    Finally
        Footprint.GroupIterator_Destroy(Iter);
        Seen.Free;
    End;
End;

Procedure AlignFootprint(Source, Target : IPCB_LibComponent);
Var Iter : IPCB_GroupIterator; Prim : IPCB_Primitive;
    DX, DY : TCoord; Seen : TStringList; Key : String;
Begin
    DX := Target.X - Source.X;
    DY := Target.Y - Source.Y;
    If (DX = 0) And (DY = 0) Then Exit;
    Seen := TStringList.Create;
    Iter := Target.GroupIterator_Create;
    Try
        Prim := Iter.FirstPCBObject;
        While Prim <> Nil Do
        Begin
            Key := IntToStr(Prim.I_ObjectAddress);
            If Seen.IndexOf(Key) < 0 Then
            Begin
                Seen.Add(Key);
                Prim.MoveByXY(DX, DY);
            End;
            Prim := Iter.NextPCBObject;
        End;
    Finally
        Target.GroupIterator_Destroy(Iter);
        Seen.Free;
    End;
End;

Procedure AttachStep(Lib : IPCB_Library; Footprint : IPCB_LibComponent; Request : TStringList);
Var Body : IPCB_ComponentBody; Model : IPCB_Model; Path : String;
Begin
    Path := DecodeText(Request.Values['step']);
    If Not FileExists(Path) Then Exit;
    Body := PCBServer.PCBObjectFactory(eComponentBodyObject, eNoDimension, eCreate_Default);
    If Body = Nil Then Fail('Cannot create 3D body');
    Model := Body.ModelFactory_FromFilename(Path, False);
    If Model = Nil Then Fail('AD could not import the STEP model');
    Model.SetState(StrToInt(Request.Values['rx']) / 1000.0,
                   StrToInt(Request.Values['ry']) / 1000.0,
                   StrToInt(Request.Values['rz']) / 1000.0,
                   StrToInt(Request.Values['z']));
    Body.SetState_FromModel;
    Body.Model := Model;
    Body.MoveByXY(StrToInt(Request.Values['x']) + Footprint.X,
                  StrToInt(Request.Values['y']) + Footprint.Y);
    Footprint.AddPCBObject(Body);
    Lib.Board.AddPCBObject(Body);
    PCBServer.SendMessageToRobots(Footprint.I_ObjectAddress, c_Broadcast,
        PCBM_BoardRegisteration, Body.I_ObjectAddress);
    PCBServer.SendMessageToRobots(Lib.Board.I_ObjectAddress, c_Broadcast,
        PCBM_BoardRegisteration, Body.I_ObjectAddress);
End;

Function HasModel(Footprint : IPCB_LibComponent) : Boolean;
Var Iter : IPCB_GroupIterator; Body : IPCB_ComponentBody;
Begin
    Result := False;
    Iter := Footprint.GroupIterator_Create;
    Try
        Iter.AddFilter_ObjectSet(MkSet(eComponentBodyObject));
        Body := Iter.FirstPCBObject;
        While Body <> Nil Do
        Begin
            If Body.Model <> Nil Then Begin Result := True; Exit; End;
            Body := Iter.NextPCBObject;
        End;
    Finally Footprint.GroupIterator_Destroy(Iter); End;
End;

Procedure SetFootprintLink(Impl : ISch_Implementation; PcbPath, Name : String);
Begin
    Impl.ModelType := 'PCBLIB';
    Impl.ModelName := Name;
    Impl.IsCurrent := True;
    Impl.UseComponentLibrary := False;
    Impl.ClearAllDatafileLinks;
    { An empty location at creation avoids a modal path lookup in AD26.
      Then explicitly bind the existing link to the selected paired library. }
    Impl.AddDataFileLink(Name, '', 'PCBLib');
    If Impl.DatafileLinkCount <> 1 Then Fail('AD could not create the footprint link');
    Impl.DatafileLink[0].Location := PcbPath;
End;

Procedure SetNewFootprintLocation(Comp : ISch_Component; PcbPath, Name : String);
Var Iter : ISch_Iterator; Impl : ISch_Implementation; Found : Boolean;
Begin
    Found := False;
    Iter := Comp.SchIterator_Create;
    If Iter = Nil Then Fail('Cannot enumerate new symbol models');
    Try
        Iter.AddFilter_ObjectSet(MkSet(eImplementation));
        Impl := Iter.FirstSchObject;
        While Impl <> Nil Do
        Begin
            If UpperCase(Impl.ModelType) = 'PCBLIB' Then
            Begin
                SetFootprintLink(Impl, PcbPath, Name);
                Found := True;
            End;
            Impl := Iter.NextSchObject;
        End;
    Finally Comp.SchIterator_Destroy(Iter); End;
    If Not Found Then
    Begin
        Impl := Comp.AddSchImplementation;
        If Impl = Nil Then Fail('AD could not create the symbol implementation');
        SetFootprintLink(Impl, PcbPath, Name);
    End;
End;

Procedure SaveAndReopen(Kind, Path : String);
Var Doc : IServerDocument;
Begin
    Doc := Client.GetDocumentByPath(Path);
    If Doc = Nil Then Fail('Cannot find edited document: ' + Path);
    Doc.SetModified(True);
    Doc.DoFileSave(Kind);
    If Doc.Modified Then Fail('AD did not save the library: ' + Path);
    If Not FileExists(Path) Then Fail('Saved library is missing: ' + Path);
    { Closing only a saved target invalidates stale library name/bounds caches.
      No schematic/PCB design documents are touched. }
    Doc := Nil;
    CloseCleanLibrary(Path);
    Doc := OpenLibrary(Kind, Path);
End;

Procedure ExecuteSync(Request, Reply : TStringList);
Var
    SchPath, PcbPath, Job, Name, Legacy, FpName, ActualName : String;
    SourceSch, TargetSch : ISch_Lib;
    SourceComp, ExistingComp, NewComp, VerifyComp : ISch_Component;
    SourcePcb, TargetPcb : IPCB_Library;
    SourceFp, ExistingFp, NewFp, VerifyFp : IPCB_LibComponent;
    Doc : IServerDocument;
    AddedSch, AddedPcb, SchCreated, PcbCreated : Boolean;
    ExpectedPrimitives, ActualPrimitives : Integer;
Begin
    LastFailure := 'AD API error during sync; see the last recorded phase';
    Name := DecodeText(Request.Values['symbol']);
    Legacy := DecodeText(Request.Values['legacy_symbol']);
    FpName := DecodeText(Request.Values['footprint']);
    SchPath := DecodeText(Request.Values['target_sch']);
    PcbPath := DecodeText(Request.Values['target_pcb']);
    Job := DecodeText(Request.Values['job']);
    AddedSch := False; AddedPcb := False;
    SchCreated := False; PcbCreated := False;
    NewComp := Nil; NewFp := Nil; TargetSch := Nil; TargetPcb := Nil;
    Try
        Phase(Request, 'preflight');
        If Name <> '' Then RequireClean(SchPath);
        If FpName <> '' Then RequireClean(PcbPath);
        { Validate sources and original documents before creating/backing up targets. }
        SourceComp := Nil; SourceFp := Nil;
        If Name <> '' Then
        Begin
            SourceSch := OpenSch(DecodeText(Request.Values['source_sch']));
            SourceComp := FindSymbol(SourceSch, Name);
            If SourceComp = Nil Then Fail('Source symbol missing');
        End;
        If FpName <> '' Then
        Begin
            SourcePcb := OpenPcb(DecodeText(Request.Values['source_pcb']));
            SourceFp := SourcePcb.GetComponentByName(FpName);
            If SourceFp = Nil Then Fail('Source footprint missing');
            ExpectedPrimitives := PrimitiveCount(SourceFp);
        End;
        If Name <> '' Then
        Begin
            { A clean open document may still cache an older disk version from
              a prior file-sync session. Never save that stale copy back. }
            CloseCleanLibrary(SchPath);
            If FileExists(SchPath) Then CopyFileBytes(SchPath, Job + '\before.SchLib')
            Else
            Begin
                ForceDirectories(ExtractFilePath(SchPath));
                CopyFileBytes(DecodeText(Request.Values['seed_sch']), SchPath);
                SchCreated := True;
            End;
            TargetSch := OpenSch(SchPath);
            RequireClean(SchPath);
        End;
        If FpName <> '' Then
        Begin
            CloseCleanLibrary(PcbPath);
            If FileExists(PcbPath) Then CopyFileBytes(PcbPath, Job + '\before.PcbLib')
            Else
            Begin
                ForceDirectories(ExtractFilePath(PcbPath));
                CopyFileBytes(DecodeText(Request.Values['seed_pcb']), PcbPath);
                PcbCreated := True;
            End;
            TargetPcb := OpenPcb(PcbPath);
            RequireClean(PcbPath);
        End;
        { A large open may consume the deadline. Check again before any edit. }
        If (Now - FileDateToDateTime(FileAge(Job + '\request.ini'))) * 86400 > StrToInt(Request.Values['ttl_seconds']) Then
            Fail('Request expired before editing; retry from the workstation');
        ExistingComp := Nil; ExistingFp := Nil;
        If Name <> '' Then
        Begin
            ExistingComp := FindTargetSymbol(TargetSch, Name, Legacy);
        End;
        If FpName <> '' Then ExistingFp := TargetPcb.GetComponentByName(FpName);
        If (Name <> '') And (ExistingComp = Nil) Then
        Begin
            Doc := OpenLibrary('SchLib', DecodeText(Request.Values['source_sch']));
            NewComp := SourceComp.Replicate;
            If NewComp = Nil Then Fail('AD could not copy source symbol');
        End;
        If ExistingFp = Nil Then
        Begin
            If FpName <> '' Then
            Begin
                Phase(Request, 'copy_footprint');
                Doc := OpenLibrary('PcbLib', PcbPath);
                PCBServer.PreProcess;
                Try
                    NewFp := TargetPcb.CreateNewComponent;
                    If NewFp = Nil Then Fail('AD could not create target footprint');
                    AddedPcb := True;
                    SourceFp.CopyTo(NewFp, eFullCopy);
                    AlignFootprint(SourceFp, NewFp);
                    NewFp.Name := FpName;
                    TargetPcb.RegisterComponent(NewFp);
                    ActualPrimitives := PrimitiveCount(NewFp);
                    If ActualPrimitives <> ExpectedPrimitives Then
                        Fail('Footprint copy primitive count mismatch');
                    AttachStep(TargetPcb, NewFp, Request);
                    TargetPcb.CurrentComponent := NewFp;
                    TargetPcb.Board.ViewManager_FullUpdate;
                Finally PCBServer.PostProcess; End;
            End;
        End;
        If (Name <> '') And (ExistingComp = Nil) Then
        Begin
            Phase(Request, 'copy_symbol');
            Doc := OpenLibrary('SchLib', SchPath);
            SchServer.ProcessControl.PreProcess(TargetSch, '');
            Try
                If FpName <> '' Then SetNewFootprintLocation(NewComp, PcbPath, FpName);
                TargetSch.AddSchComponent(NewComp);
                AddedSch := True;
                NewComp.LibReference := Name;
                NewComp.DesignItemId := Name;
                SchServer.RobotManager.SendMessage(Nil, Nil, SCHM_PrimitiveRegistration, NewComp.I_ObjectAddress);
                TargetSch.CurrentSchComponent := NewComp;
                TargetSch.GraphicallyInvalidate;
            Finally SchServer.ProcessControl.PostProcess(TargetSch, 'Edit'); End;
        End;
        Phase(Request, 'saving');
        { Drop design-object references before closing their owning documents. }
        NewFp := Nil; NewComp := Nil; ExistingFp := Nil; ExistingComp := Nil;
        TargetPcb := Nil; TargetSch := Nil;
        If AddedPcb Or PcbCreated Then
        Begin SaveAndReopen('PcbLib', PcbPath); End;
        If AddedSch Or SchCreated Then
        Begin SaveAndReopen('SchLib', SchPath); End;
        Phase(Request, 'verifying');
        ActualName := '';
        If Name <> '' Then
        Begin
            TargetSch := OpenSch(SchPath);
            VerifyComp := FindTargetSymbol(TargetSch, Name, Legacy);
            If VerifyComp = Nil Then Fail('Saved symbol could not be read back by AD');
            ActualName := VerifyComp.LibReference;
            TargetSch.CurrentSchComponent := VerifyComp;
            TargetSch.GraphicallyInvalidate;
        End;
        If FpName <> '' Then
        Begin
            TargetPcb := OpenPcb(PcbPath);
            VerifyFp := TargetPcb.GetComponentByName(FpName);
            If VerifyFp = Nil Then Fail('Saved footprint could not be read back by AD');
            If AddedPcb And FileExists(DecodeText(Request.Values['step'])) Then
            Begin
                If Not HasModel(VerifyFp) Then Fail('Saved footprint has no readable 3D model');
                Reply.Values['model_verified'] := '1';
            End;
            TargetPcb.CurrentComponent := VerifyFp;
            TargetPcb.Board.ViewManager_FullUpdate;
        End;
        If Name <> '' Then
        Begin
            Doc := OpenLibrary('SchLib', SchPath);
            TargetSch.GraphicallyInvalidate;
        End;
        Reply.Values['component'] := EncodeText(ActualName);
        Reply.Values['footprint'] := EncodeText(FpName);
        If Not AddedSch Then Reply.Values['existing'] := '1' Else Reply.Values['existing'] := '0';
        If Not AddedPcb Then Reply.Values['footprint_existing'] := '1' Else Reply.Values['footprint_existing'] := '0';
        Reply.Values['verified'] := '1';
        Reply.Values['ok'] := '1';
        Phase(Request, 'complete');
    Except
        { Keep saved partial results and backups. Never restore an entire file
          behind AD's in-memory document, and never touch existing components. }
        Reply.Values['ok'] := '0';
        Reply.Values['verified'] := '0';
        Reply.Values['error'] := EncodeText(LastFailure + ' (phase details and pre-sync backups: ' + Job + ')');
        StatusLabel.Caption := 'Failed. See workstation task details. Save/review AD edits before retrying.';
    End;
    { Close only our disposable, clean source documents. }
    Try
        Doc := Client.GetDocumentByPath(DecodeText(Request.Values['source_sch']));
        If Doc <> Nil Then If Not Doc.Modified Then Client.CloseDocument(Doc);
        Doc := Client.GetDocumentByPath(DecodeText(Request.Values['source_pcb']));
        If Doc <> Nil Then If Not Doc.Modified Then Client.CloseDocument(Doc);
    Except End;
    { All saved targets stay open. No stale document interfaces are reused. }
End;

Procedure PollTimerTimer(Sender : TObject);
Var Request, Reply : TStringList; Pending, Job : String;
Begin
    If InTick Or Not BridgeRunning Then Exit;
    InTick := True;
    PollTimer.Enabled := False;
    Request := TStringList.Create;
    Reply := TStringList.Create;
    Try
        Try
            If FileExists(IpcRoot + '\stop') Then
            Begin
                BridgeRunning := False;
                Heartbeat('stopped');
                StatusLabel.Caption := 'Disconnected. File sync requires closing AD.';
                Exit;
            End;
            Heartbeat('ready');
            Pending := IpcRoot + '\pending.ini';
            If Not FileExists(Pending) Then Exit;
            Request.LoadFromFile(Pending);
            If Request.Values['protocol'] <> '1' Then Fail('Unsupported bridge protocol');
            If Request.Values['session'] <> SessionId Then
            Begin DeleteFile(Pending); Exit; End;
            Job := DecodeText(Request.Values['job']);
            If Pos(UpperCase(IpcRoot + '\jobs\'), UpperCase(Job + '\')) <> 1 Then
                Fail('Request job is outside the bridge directory');
            Reply.Values['protocol'] := '1';
            Reply.Values['session'] := SessionId;
            Reply.Values['request'] := Request.Values['request'];
            Heartbeat('busy');
            Phase(Request, 'opening');
            If Not DeleteFile(Pending) Then Fail('Cannot claim sync request');
            If (Now - FileDateToDateTime(FileAge(Job + '\request.ini'))) * 86400 > StrToInt(Request.Values['ttl_seconds']) Then
            Begin
                Reply.Values['ok'] := '0';
                Reply.Values['error'] := EncodeText('Request expired without editing the target library');
            End
            Else ExecuteSync(Request, Reply);
            WriteFields(Job + '\reply.ini', Reply);
        Except
            BridgeRunning := False;
            StatusLabel.Caption := 'Disconnected: ' + LastFailure;
            Heartbeat('stopped');
        End;
    Finally
        Request.Free;
        Reply.Free;
        InTick := False;
        If BridgeRunning Then
        Begin
            Heartbeat('ready');
            PollTimer.Enabled := True;
        End;
    End;
End;

Procedure LcedaBridgeFormClose(Sender : TObject; Var Action : TCloseAction);
Begin
    BridgeRunning := False;
    PollTimer.Enabled := False;
    If IpcRoot <> '' Then Heartbeat('stopped');
End;

Procedure StopButtonClick(Sender : TObject);
Begin
    LcedaBridgeForm.Close;
End;

Procedure LCEDA_StartBridge;
Var Prior : TStringList; Path : String;
Begin
    If BridgeRunning Then Exit;
    IpcRoot := DecodeText('__IPC_ROOT_HEX__');
    Path := IpcRoot + '\heartbeat.ini';
    If FileExists(Path) Then
    Begin
        Prior := TStringList.Create;
        Try
            Prior.LoadFromFile(Path);
            If (Prior.Values['state'] = 'ready') And
               ((Now - FileDateToDateTime(FileAge(Path))) * 86400 < 4) Then
                Fail('Another AD connection is already active for this user');
            If (Prior.Values['state'] = 'busy') And
               ((Now - FileDateToDateTime(FileAge(Path))) * 86400 < 130) Then
                Fail('An AD library sync is still active for this user');
        Finally Prior.Free; End;
    End;
    If FileExists(IpcRoot + '\stop') Then DeleteFile(IpcRoot + '\stop');
    SessionId := IntToStr(GetTickCount) + '-' + FormatDateTime('yyyymmddhhnnsszzz', Now);
    InTick := False;
    BridgeRunning := True;
    StatusLabel.Caption := 'Connected. Return to JLC Export Workstation to sync your bound libraries.';
    LcedaBridgeForm.Show;
    Heartbeat('ready');
    PollTimer.Enabled := True;
End;
