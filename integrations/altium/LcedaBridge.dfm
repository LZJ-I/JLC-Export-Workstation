object LcedaBridgeForm: TLcedaBridgeForm
  Left = 200
  Top = 200
  Caption = 'JLC Export Workstation - AD connection'
  ClientHeight = 132
  ClientWidth = 460
  BorderStyle = bsToolWindow
  Position = poScreenCenter
  OnClose = LcedaBridgeFormClose
  object StatusLabel: TLabel
    Left = 16
    Top = 16
    Width = 428
    Height = 72
    AutoSize = False
    WordWrap = True
    Caption = 'Ready. Return to JLC Export Workstation to sync your bound libraries.'
  end
  object StopButton: TButton
    Left = 340
    Top = 96
    Width = 104
    Height = 28
    Caption = 'Disconnect'
    OnClick = StopButtonClick
  end
  object PollTimer: TTimer
    Enabled = False
    Interval = 400
    OnTimer = PollTimerTimer
    Left = 24
    Top = 92
  end
end
