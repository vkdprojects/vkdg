# Run this with: vhs demo.tape
# Install VHS: brew install charmbracelet/tap/vhs

Output assets/demo.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 500
Set Padding 24
Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set PlaybackSpeed 1.0

# Clear and set the scene
Type "# Start VKDG with an API key"
Sleep 500ms
Enter
Sleep 300ms

Type "export ANTHROPIC_API_KEY=sk-ant-demo"
Enter
Sleep 300ms

Type "vkdg serve"
Enter
Sleep 2s

# The banner appears — let it breathe
Sleep 3s

# Ctrl+C to stop
Ctrl+C
Sleep 800ms

Type "# Run a quick health check"
Enter
Sleep 300ms

Type "curl -s localhost:8080/health | python3 -m json.tool"
Enter
Sleep 1.5s

Sleep 2s
