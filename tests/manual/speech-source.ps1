# Development-only English TTS source. No microphone, capture, files or network.
# Run with Windows PowerShell 5.1, which supplies the Windows desktop assemblies.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Speech

$fixtureSynth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$fixtureVoices = @($fixtureSynth.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq 'en' })
if ($fixtureVoices.Count -eq 0) {
    $fixtureSynth.Dispose()
    throw 'No installed English Windows TTS voice. Install/select a voice manually before this optional test.'
}
$fixtureSynth.SelectVoice($fixtureVoices[0].VoiceInfo.Name)
$fixtureSynth.SetOutputToDefaultAudioDevice()
$fixtureSynth.Volume = 20
$fixtureText = 'This is a synthetic subtitle test. The software and server are working. The next sentence checks synchronization and translation.'

$fixtureForm = New-Object System.Windows.Forms.Form
$fixtureForm.Text = 'SubtitleNow synthetic English speech - PID ' + $PID
$fixtureForm.ClientSize = New-Object System.Drawing.Size(570, 200)
$fixtureForm.StartPosition = 'CenterScreen'
$fixtureForm.FormBorderStyle = 'FixedDialog'
$fixtureForm.MaximizeBox = $false
$fixtureLabel = New-Object System.Windows.Forms.Label
$fixtureLabel.AutoSize = $false
$fixtureLabel.Location = New-Object System.Drawing.Point(16, 15)
$fixtureLabel.Size = New-Object System.Drawing.Size(540, 90)
$fixtureLabel.Text = "Development test source PID: $PID`r`nEnglish Windows TTS only. No microphone, recording, files or cloud.`r`nSelect this PID in SubtitleNow. Start/stop below controls only this test.`r`nSystem/device volume is unchanged. Closing this window releases the source."
$fixtureStart = New-Object System.Windows.Forms.Button
$fixtureStart.Text = 'Start synthetic speech'
$fixtureStart.Location = New-Object System.Drawing.Point(16, 120)
$fixtureStart.Size = New-Object System.Drawing.Size(210, 35)
$fixtureStop = New-Object System.Windows.Forms.Button
$fixtureStop.Text = 'Stop synthetic speech'
$fixtureStop.Location = New-Object System.Drawing.Point(245, 120)
$fixtureStop.Size = New-Object System.Drawing.Size(210, 35)
$fixtureStatus = New-Object System.Windows.Forms.Label
$fixtureStatus.AutoSize = $true
$fixtureStatus.Location = New-Object System.Drawing.Point(16, 168)
$fixtureStatus.Text = 'Stopped. Press Start to create an active audio session.'
$fixtureTimer = New-Object System.Windows.Forms.Timer
$fixtureTimer.Interval = 12000
$fixtureTimer.Add_Tick({
    if ($fixtureSynth.State -ne [System.Speech.Synthesis.SynthesizerState]::Speaking) {
        $null = $fixtureSynth.SpeakAsync($fixtureText)
    }
})
$fixtureStart.Add_Click({
    $fixtureSynth.SpeakAsyncCancelAll()
    $null = $fixtureSynth.SpeakAsync($fixtureText)
    $fixtureTimer.Start()
    $fixtureStatus.Text = 'Playing synthetic English speech. Select PID ' + $PID + ' in SubtitleNow.'
})
$fixtureStop.Add_Click({
    $fixtureTimer.Stop()
    $fixtureSynth.SpeakAsyncCancelAll()
    $fixtureStatus.Text = 'Stopped. Capture should become silent.'
})
$fixtureForm.Controls.AddRange(@($fixtureLabel, $fixtureStart, $fixtureStop, $fixtureStatus))
try {
    $null = $fixtureForm.ShowDialog()
} finally {
    $fixtureTimer.Stop()
    $fixtureTimer.Dispose()
    $fixtureSynth.SpeakAsyncCancelAll()
    $fixtureSynth.Dispose()
    $fixtureForm.Dispose()
}
