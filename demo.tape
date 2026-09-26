# Run this with: vhs demo.tape
# Install VHS: brew install charmbracelet/tap/vhs

Output assets/demo.gif

Set Theme "Catppuccin Mocha"
Set FontSize 13
Set Width 920
Set Height 480
Set Padding 20
Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set PlaybackSpeed 0.9

# Setup PATH to include vkdg binary
Env PATH "/tmp:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"

Sleep 200ms

# Show the one-liner install concept
Type "# Deploy your AI gateway"
Sleep 600ms
Enter
Sleep 400ms

Type "export ANTHROPIC_API_KEY=sk-ant-..."
Enter
Sleep 400ms

Type "vkdg serve"
Sleep 400ms
Enter
Sleep 2.5s

# Let the banner breathe
Sleep 3s

# Show it's actually serving
Ctrl+C
Sleep 600ms

Type "curl -s localhost:8080/health"
Enter
Sleep 1.2s

Sleep 2s
