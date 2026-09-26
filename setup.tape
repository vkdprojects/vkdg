# vkdg setup demo — run with: vhs setup.tape
# Requires vkdg in PATH

Output assets/demo-setup.gif

Set Theme    "Catppuccin Mocha"
Set FontSize  13
Set Width     900
Set Height    600
Set Padding   20
Set Shell     "bash"
Set FontFamily "JetBrains Mono"
Set PlaybackSpeed 0.9
Set TypingSpeed 60ms

Sleep 300ms

# Show the setup command
Type "vkdg setup"
Sleep 400ms
Enter
Sleep 1000ms

# Provider selection — arrow down to Groq (free tier), then select
Sleep 1200ms
Down
Sleep 400ms
Down
Sleep 400ms
Enter
Sleep 800ms

# Env var — keep default GROQ_API_KEY
Enter
Sleep 600ms

# Model selection — keep default
Enter
Sleep 600ms

# Config path — keep default
Enter
Sleep 2500ms

# Let the spinner + completion breathe
Sleep 4000ms
