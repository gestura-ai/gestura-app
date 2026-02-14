# Gestura.app User Manual

## Table of Contents

1. [Getting Started](#getting-started)
2. [Installation](#installation)
3. [First-Time Setup](#first-time-setup)
4. [Voice Control](#voice-control)
5. [Gesture Control](#gesture-control)
6. [Haptic Harmony Ring](#haptic-harmony-ring)
7. [Customization](#customization)
8. [Troubleshooting](#troubleshooting)
9. [Advanced Features](#advanced-features)
10. [Privacy & Security](#privacy--security)

## Getting Started

Welcome to Gestura.app! This revolutionary application transforms how you interact with your computer using voice commands and hand gestures through the Haptic Harmony ring.

### What You Can Do

- **Voice Control**: Convert speech to text, execute voice commands
- **Gesture Control**: Control applications with hand movements
- **Haptic Feedback**: Receive tactile confirmation through the ring
- **Custom Automation**: Create personalized workflows
- **Multi-Platform**: Works on macOS, Windows, and Linux

### System Requirements

**Minimum Requirements:**
- **OS**: macOS 10.15+, Windows 10+, or Ubuntu 18.04+
- **RAM**: 4GB
- **Storage**: 500MB free space
- **Microphone**: Built-in or external microphone
- **Bluetooth**: Bluetooth 5.0+ for ring connectivity

**Recommended:**
- **RAM**: 8GB or more
- **CPU**: Multi-core processor
- **Microphone**: High-quality external microphone
- **Haptic Harmony Ring**: For full gesture functionality

## Installation

### macOS

1. Download `Gestura.dmg` from [gestura.app](https://gestura.app)
2. Double-click the DMG file
3. Drag Gestura.app to Applications folder
4. Right-click and select "Open" (first time only)
5. Grant microphone and accessibility permissions

### Windows

1. Download `Gestura-Setup.exe`
2. Run the installer as Administrator
3. Follow the installation wizard
4. Grant microphone permissions when prompted
5. Launch from Start Menu or Desktop

### Linux

**Using .deb package (Ubuntu/Debian):**
```bash
wget https://releases.gestura.app/latest/gestura.deb
sudo dpkg -i gestura.deb
sudo apt-get install -f  # Fix dependencies if needed
```

**Using .rpm package (Fedora/CentOS):**
```bash
wget https://releases.gestura.app/latest/gestura.rpm
sudo rpm -i gestura.rpm
```

**Using AppImage:**
```bash
wget https://releases.gestura.app/latest/Gestura.AppImage
chmod +x Gestura.AppImage
./Gestura.AppImage
```

## First-Time Setup

### Onboarding Wizard

When you first launch Gestura.app, the onboarding wizard will guide you through setup:

#### Step 1: Welcome & Permissions
- Grant microphone access
- Grant accessibility permissions (for system control)
- Grant Bluetooth access (for ring connectivity)

#### Step 2: Voice Training
- Read the provided text samples (5-10 sentences)
- Speak clearly in your normal voice
- Training improves recognition accuracy by 30-50%

#### Step 3: Ring Pairing (Optional)
- Turn on your Haptic Harmony ring
- Put ring in pairing mode (hold button for 3 seconds)
- Select your ring from the device list
- Complete pairing process

#### Step 4: Gesture Calibration
- Follow on-screen instructions for basic gestures
- Perform each gesture 3-5 times
- System learns your unique gesture patterns

#### Step 5: Preferences
- Choose default language
- Set voice command activation method
- Configure privacy settings
- Select startup preferences

### Quick Start Tutorial

After setup, try these basic commands:

**Voice Commands:**
- "Hello Gestura" - Activate voice mode
- "Type hello world" - Dictate text
- "Open calculator" - Launch applications
- "What time is it?" - Get information

**Basic Gestures:**
- **Tap**: Single finger tap (click)
- **Double Tap**: Quick double tap (double-click)
- **Swipe Left/Right**: Navigate between items
- **Pinch**: Zoom in/out
- **Hold**: Context menu

## Voice Control

### Activation Methods

**Wake Word**: Say "Hello Gestura" to activate
**Push-to-Talk**: Hold configured key while speaking
**Always Listening**: Continuous voice recognition (privacy mode available)

### Voice Commands

#### Text Dictation
```
"Type [your text]"
"Dictate [your text]"
"Write [your text]"
```

#### Application Control
```
"Open [application name]"
"Close [application name]"
"Switch to [application name]"
"Minimize window"
"Maximize window"
```

#### System Commands
```
"Volume up/down"
"Mute/unmute"
"Lock screen"
"Sleep computer"
"Take screenshot"
```

#### Navigation
```
"Scroll up/down"
"Go back/forward"
"Refresh page"
"New tab"
"Close tab"
```

### Voice Training

Improve recognition accuracy:

1. **Settings** → **Voice** → **Training**
2. Read provided text samples
3. Speak in different environments (quiet, noisy)
4. Add custom vocabulary for specialized terms
5. Review and correct misrecognitions

### Language Support

Supported languages:
- English (US, UK, AU, CA)
- Spanish (ES, MX, AR)
- French (FR, CA)
- German (DE, AT, CH)
- Italian (IT)
- Portuguese (BR, PT)
- Japanese (JP)
- Korean (KR)
- Chinese (CN, TW, HK)

## Gesture Control

### Basic Gestures

#### Tap Gestures
- **Single Tap**: Left click
- **Double Tap**: Double click
- **Triple Tap**: Custom action
- **Long Press**: Right click/context menu

#### Swipe Gestures
- **Swipe Left**: Back/Previous
- **Swipe Right**: Forward/Next
- **Swipe Up**: Scroll up/Page up
- **Swipe Down**: Scroll down/Page down

#### Pinch Gestures
- **Pinch In**: Zoom out/Decrease
- **Pinch Out**: Zoom in/Increase
- **Rotate**: Rotate content

#### Advanced Gestures
- **Circle**: Custom action
- **Figure-8**: Custom action
- **Shake**: Undo
- **Tilt**: Adjust settings

### Gesture Customization

Create custom gestures:

1. **Settings** → **Gestures** → **Custom Gestures**
2. Click **"Create New Gesture"**
3. Name your gesture
4. Record gesture pattern (perform 5 times)
5. Assign action or command
6. Test and refine

### Gesture Sensitivity

Adjust sensitivity for better recognition:

- **High Sensitivity**: Detects subtle movements
- **Medium Sensitivity**: Balanced (recommended)
- **Low Sensitivity**: Requires deliberate movements

## Haptic Harmony Ring

### Ring Setup

#### Initial Pairing
1. Charge ring for 2+ hours
2. Press and hold power button (3 seconds)
3. Ring LED flashes blue (pairing mode)
4. In Gestura: **Settings** → **Ring** → **Pair Device**
5. Select your ring from list
6. Complete pairing

#### Ring Status
Monitor ring status in the app:
- **Battery Level**: 0-100%
- **Connection Status**: Connected/Disconnected
- **Signal Strength**: -30 to -90 dBm
- **Firmware Version**: Current version

### Ring Features

#### Haptic Feedback Patterns
- **Single Pulse**: Confirmation
- **Double Pulse**: Warning/Error
- **Long Vibration**: Notification
- **Rhythmic Pattern**: Custom alerts

#### Gesture Recognition
The ring provides precise gesture data:
- **9-axis IMU**: Accelerometer, gyroscope, magnetometer
- **High Sampling Rate**: 100Hz for smooth tracking
- **Low Latency**: <50ms response time

### Ring Maintenance

#### Battery Care
- Charge when battery drops below 20%
- Full charge takes 1-2 hours
- Battery lasts 8-12 hours with normal use
- Use provided magnetic charger only

#### Cleaning
- Wipe with damp cloth
- Avoid harsh chemicals
- Dry thoroughly before charging
- Store in provided case

#### Troubleshooting
- **Won't pair**: Reset ring (hold button 10 seconds)
- **Poor connection**: Check Bluetooth range (<10 meters)
- **Inaccurate gestures**: Recalibrate in settings
- **Battery drains fast**: Check for interference

## Customization

### Voice Customization

#### Custom Commands
Create personalized voice commands:

1. **Settings** → **Voice** → **Custom Commands**
2. Click **"Add Command"**
3. Enter trigger phrase
4. Choose action type:
   - Launch application
   - Type text
   - Execute system command
   - Run script
5. Test command

#### Voice Profiles
Create profiles for different users:
- **Personal Profile**: Your voice patterns
- **Family Profiles**: Other household members
- **Work Profile**: Professional vocabulary
- **Gaming Profile**: Game-specific commands

### Gesture Customization

#### Application-Specific Gestures
Configure gestures for specific apps:

1. **Settings** → **Gestures** → **App-Specific**
2. Select application
3. Assign gestures to app functions
4. Test in application

#### Gesture Combinations
Create complex gesture sequences:
- **Gesture + Voice**: Combined input
- **Multi-Gesture**: Sequence of gestures
- **Conditional Gestures**: Context-dependent actions

### Interface Customization

#### Themes
- **Light Theme**: Default bright interface
- **Dark Theme**: Easy on eyes
- **High Contrast**: Accessibility option
- **Custom Theme**: Create your own

#### Layout Options
- **Compact View**: Minimal interface
- **Standard View**: Default layout
- **Expanded View**: Maximum information
- **Custom Layout**: Arrange panels

## Troubleshooting

### Common Issues

#### Voice Recognition Problems

**Issue**: Voice commands not recognized
**Solutions**:
- Check microphone permissions
- Reduce background noise
- Retrain voice model
- Adjust microphone sensitivity
- Update audio drivers

**Issue**: Poor recognition accuracy
**Solutions**:
- Complete voice training
- Speak clearly and at normal pace
- Add custom vocabulary
- Check microphone quality
- Adjust language settings

#### Gesture Recognition Problems

**Issue**: Gestures not detected
**Solutions**:
- Check ring battery level
- Verify Bluetooth connection
- Recalibrate gestures
- Check for interference
- Update ring firmware

**Issue**: False gesture detection
**Solutions**:
- Adjust sensitivity settings
- Retrain gesture patterns
- Check ring fit (not too loose/tight)
- Minimize hand movements when not gesturing

#### Connection Issues

**Issue**: Ring won't connect
**Solutions**:
- Reset ring (hold button 10 seconds)
- Clear Bluetooth cache
- Restart Gestura.app
- Check Bluetooth is enabled
- Move closer to computer

**Issue**: Frequent disconnections
**Solutions**:
- Check battery level
- Reduce distance to computer
- Remove Bluetooth interference
- Update Bluetooth drivers
- Reset network settings

### Performance Issues

#### High CPU Usage
- Close unnecessary applications
- Reduce voice recognition frequency
- Disable unused features
- Update to latest version
- Check for malware

#### High Memory Usage
- Restart application
- Clear cache files
- Reduce history retention
- Close other memory-intensive apps
- Add more RAM if needed

### Getting Help

#### Built-in Help
- **Help Menu**: Comprehensive guides
- **Tooltips**: Hover over interface elements
- **Tutorials**: Interactive walkthroughs
- **FAQ**: Common questions answered

#### Online Resources
- **Documentation**: https://docs.gestura.app
- **Video Tutorials**: https://youtube.com/gestura
- **Community Forum**: https://community.gestura.app
- **Discord**: https://discord.gg/gestura

#### Support Channels
- **Email**: support@gestura.app
- **Live Chat**: Available in app
- **Phone**: 1-800-GESTURA (premium users)
- **Remote Assistance**: Screen sharing support

## Advanced Features

### Chat Window (Agentic Coding)

Gestura includes a dedicated **Chat** window for text-based and voice-assisted workflows. The chat UI can also display a project-scoped file explorer so you can reference your workspace while chatting.

#### Project Explorer Panel

- Open via the **folder** button below the input, or press **Cmd/Ctrl+B**.
- The explorer is rooted at **Project Root** (the current chat session workspace) and does not allow browsing outside that directory.
- If the workspace is a git repository, changed files/folders show small status badges; otherwise the panel shows **"Not a git repository."**
- Use the **Refresh** button in the Explorer header to reload the tree.

#### Chat Window Shortcuts

- **Cmd/Ctrl+B**: Toggle Explorer
- **Cmd/Ctrl+T**: Toggle Tasks panel
- **Cmd/Ctrl+S**: Toggle Knowledge/Skills panel
- **Cmd/Ctrl+K** (in the message box): Enhance prompt
- **Cmd/Ctrl+Z** (after enhance): Undo enhancement

### Automation & Scripting

#### Workflow Automation
Create complex automation workflows:

1. **Settings** → **Automation** → **New Workflow**
2. Choose trigger (voice, gesture, time, event)
3. Add actions (sequence of commands)
4. Set conditions (if/then logic)
5. Test and activate

#### Scripting Support
Write custom scripts in multiple languages:

**Lua Example**:
```lua
-- Custom gesture action
function onGesture(gestureType)
    if gestureType == "circle" then
        app.notify("Circle gesture detected!")
        app.execute("open calculator")
    end
end
```

**Python Example**:
```python
# Voice command handler
def handle_voice_command(text):
    if "weather" in text.lower():
        weather = get_weather()
        speak(f"Today's weather is {weather}")
```

### Plugin System

#### Installing Plugins
1. **Settings** → **Plugins** → **Browse**
2. Search plugin marketplace
3. Click **"Install"** on desired plugin
4. Grant requested permissions
5. Configure plugin settings

#### Popular Plugins
- **Smart Home**: Control IoT devices
- **Productivity**: Calendar, tasks, notes
- **Entertainment**: Music, video control
- **Development**: Code snippets, terminal
- **Accessibility**: Enhanced accessibility features

### Integration APIs

#### Third-Party Integrations
Connect with popular services:

- **Slack**: Send messages, join calls
- **Zoom**: Start/join meetings
- **Spotify**: Control music playback
- **Google Calendar**: Create events, check schedule
- **Notion**: Create notes, search content

#### Webhook Support
Configure webhooks for real-time notifications:

```json
{
  "url": "https://your-server.com/webhook",
  "events": ["voice.recognized", "gesture.detected"],
  "secret": "your-webhook-secret"
}
```

## Privacy & Security

### Data Protection

#### Local Processing
- Voice recognition runs locally
- Gesture data processed on-device
- No audio sent to cloud by default
- Encrypted local storage

#### Privacy Controls
- **Minimal Data Collection**: Only essential data
- **Opt-in Analytics**: Choose what to share
- **Data Retention**: Automatic cleanup
- **Export/Delete**: Full data control

### Security Features

#### Encryption
- **AES-256**: File encryption
- **TLS 1.3**: Network communication
- **Key Management**: Secure key storage
- **Digital Signatures**: Code integrity

#### Access Control
- **User Authentication**: Secure login
- **Permission System**: Granular controls
- **Audit Logging**: Track all actions
- **Session Management**: Secure sessions

### GDPR Compliance

#### Your Rights
- **Right to Access**: View your data
- **Right to Rectification**: Correct data
- **Right to Erasure**: Delete data
- **Right to Portability**: Export data
- **Right to Object**: Opt-out of processing

#### Data Processing
- **Lawful Basis**: Legitimate interest/consent
- **Purpose Limitation**: Specific purposes only
- **Data Minimization**: Collect only necessary data
- **Retention Limits**: Automatic deletion

### Privacy Settings

#### Voice Privacy
- **Local Processing**: Keep voice data local
- **Voice Deletion**: Auto-delete recordings
- **Anonymous Mode**: Remove identifying info
- **Opt-out Analytics**: Disable usage tracking

#### Gesture Privacy
- **Local Recognition**: Process gestures locally
- **Data Anonymization**: Remove personal patterns
- **Sharing Controls**: Control data sharing
- **Retention Settings**: Set data lifetime

---

## Support & Community

### Getting Help
- **In-App Help**: Press F1 or click Help menu
- **Online Documentation**: https://docs.gestura.app
- **Video Tutorials**: https://youtube.com/gestura
- **Community Forum**: https://community.gestura.app

### Stay Updated
- **Newsletter**: Monthly updates and tips
- **Blog**: https://blog.gestura.app
- **Social Media**: @GesturaApp
- **Release Notes**: Check for updates in app

### Feedback
We value your feedback! Share suggestions:
- **Feedback Form**: In app settings
- **Feature Requests**: community.gestura.app
- **Bug Reports**: support@gestura.app
- **User Research**: Participate in studies

---

*Thank you for choosing Gestura.app! We're excited to see how you'll use voice and gesture control to enhance your computing experience.*

---

**Version**: 1.0.0
**Last Updated**: January 2024
**Copyright**: © 2024 Gestura AI. All rights reserved.
