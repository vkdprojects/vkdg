# vkdg demo — run with: vhs demo.tape

Output assets/demo.gif

Set Theme    "Catppuccin Mocha"
Set FontSize  13
Set Width     860
Set Height    460
Set Padding   20
Set Shell     "bash"
Set FontFamily "JetBrains Mono"
Set PlaybackSpeed 1.0
Set TypingSpeed 60ms

Sleep 300ms

Type "export ANTHROPIC_API_KEY=sk-ant-api03-..."
Enter
Sleep 400ms

Type "vkdg serve --token my-secret-token"
Sleep 400ms
Enter
Sleep 2500ms

# Banner is fully rendered — let it breathe
Sleep 3500ms
