# Gestura.app Troubleshooting Guide

## Quick Diagnostics

### Built-in Diagnostic Tool

1. Open Gestura.app
2. Go to **Help** → **Diagnostics**
3. Click **"Run Full Diagnostic"**
4. Review results and follow recommendations

### System Health Check

```bash
# Check system requirements
gestura --system-check

# Test microphone
gestura --test-microphone

# Test Bluetooth
gestura --test-bluetooth

# Check permissions
gestura --check-permissions
```

## Common Issues

### Installation Problems

#### Issue: "App can't be opened" (macOS)
**Cause**: Gatekeeper security restriction
**Solution**:
1. Right-click Gestura.app
2. Select "Open"
3. Click "Open" in security dialog
4. Grant permissions when prompted

#### Issue: "Windows protected your PC" (Windows)
**Cause**: SmartScreen filter
**Solution**:
1. Click "More info"
2. Click "Run anyway"
3. Or disable SmartScreen temporarily

#### Issue: Permission denied (Linux)
**Cause**: Insufficient permissions
**Solution**:
```bash
# Make executable
chmod +x Gestura.AppImage

# Or install with package manager
sudo dpkg -i gestura.deb
sudo apt-get install -f
```

### Voice Recognition Issues

#### Issue: Voice commands not recognized
**Symptoms**:
- App doesn't respond to voice
- Low recognition accuracy
- Microphone not detected

**Diagnostic Steps**:
1. **Check microphone permissions**:
   - macOS: System Preferences → Security & Privacy → Microphone
   - Windows: Settings → Privacy → Microphone
   - Linux: Check PulseAudio/ALSA settings

2. **Test microphone**:
   ```bash
   # Test recording
   gestura --test-microphone
   
   # Check audio levels
   gestura --audio-levels
   ```

3. **Check audio settings**:
   - Verify correct input device selected
   - Adjust microphone sensitivity
   - Reduce background noise

**Solutions**:
- **Grant microphone permissions**
- **Retrain voice model**: Settings → Voice → Training
- **Adjust sensitivity**: Settings → Voice → Sensitivity
- **Change microphone**: Use external high-quality microphone
- **Update audio drivers**: Check manufacturer website

#### Issue: Poor recognition accuracy
**Symptoms**:
- Frequent misrecognitions
- Commands not understood
- Foreign accent issues

**Solutions**:
1. **Complete voice training**:
   - Settings → Voice → Training
   - Read all provided samples
   - Train in quiet environment

2. **Add custom vocabulary**:
   - Settings → Voice → Custom Words
   - Add frequently used terms
   - Include proper names, technical terms

3. **Adjust language settings**:
   - Settings → Voice → Language
   - Select correct dialect
   - Enable accent adaptation

4. **Improve audio quality**:
   - Use noise-canceling microphone
   - Reduce background noise
   - Speak clearly and at normal pace

### Gesture Recognition Issues

#### Issue: Ring not connecting
**Symptoms**:
- Ring not found during pairing
- Connection drops frequently
- "Device not found" error

**Diagnostic Steps**:
1. **Check ring status**:
   - Press ring button - LED should light up
   - Check battery level (charge if needed)
   - Verify ring is in pairing mode

2. **Check Bluetooth**:
   ```bash
   # Test Bluetooth
   gestura --test-bluetooth
   
   # Scan for devices
   gestura --scan-devices
   ```

3. **Check system Bluetooth**:
   - Ensure Bluetooth is enabled
   - Check for interference
   - Verify Bluetooth version (5.0+ required)

**Solutions**:
- **Reset ring**: Hold button for 10 seconds
- **Clear Bluetooth cache**: 
  - Windows: Device Manager → Bluetooth → Uninstall
  - macOS: Reset Bluetooth module
  - Linux: `sudo systemctl restart bluetooth`
- **Move closer**: Stay within 10 meters
- **Remove interference**: Turn off other Bluetooth devices
- **Update drivers**: Install latest Bluetooth drivers

#### Issue: Gestures not detected
**Symptoms**:
- No response to gestures
- Low detection accuracy
- False positives

**Solutions**:
1. **Recalibrate gestures**:
   - Settings → Gestures → Calibration
   - Perform each gesture 5 times
   - Ensure consistent movements

2. **Adjust sensitivity**:
   - Settings → Gestures → Sensitivity
   - Start with medium sensitivity
   - Adjust based on your movement style

3. **Check ring fit**:
   - Ring should be snug but comfortable
   - Not too loose (slips) or tight (restricts movement)
   - Wear on dominant hand's index finger

4. **Minimize interference**:
   - Remove other rings/jewelry
   - Avoid rapid hand movements when not gesturing
   - Keep hand steady between gestures

### Performance Issues

#### Issue: High CPU usage
**Symptoms**:
- Computer runs hot
- Fan noise increases
- Other apps slow down

**Solutions**:
1. **Reduce processing load**:
   - Settings → Performance → Voice Processing
   - Lower recognition frequency
   - Disable unused features

2. **Close unnecessary apps**:
   - Check Task Manager/Activity Monitor
   - Close resource-intensive applications
   - Restart Gestura.app

3. **Update hardware**:
   - Add more RAM if using <8GB
   - Use SSD instead of HDD
   - Upgrade to newer CPU

#### Issue: High memory usage
**Symptoms**:
- System runs out of memory
- Gestura.app crashes
- Slow performance

**Solutions**:
1. **Clear cache**:
   - Settings → Advanced → Clear Cache
   - Restart application
   - Clear browser cache if using web interface

2. **Reduce memory usage**:
   - Settings → Performance → Memory Optimization
   - Reduce history retention
   - Disable memory-intensive features

3. **System optimization**:
   - Close other memory-intensive apps
   - Restart computer regularly
   - Check for memory leaks in other apps

### Connection Issues

#### Issue: Ring disconnects frequently
**Symptoms**:
- Connection drops every few minutes
- "Ring disconnected" notifications
- Gestures stop working intermittently

**Solutions**:
1. **Check battery level**:
   - Charge ring if below 20%
   - Replace battery if old (contact support)

2. **Reduce interference**:
   - Move away from WiFi routers
   - Turn off other Bluetooth devices
   - Avoid metal objects between ring and computer

3. **Update firmware**:
   - Settings → Ring → Check for Updates
   - Follow update instructions
   - Restart both ring and app after update

4. **Reset connection**:
   - Settings → Ring → Forget Device
   - Re-pair ring from scratch
   - Complete calibration again

#### Issue: API connection failures
**Symptoms**:
- "Connection timeout" errors
- API requests fail
- Cloud features unavailable

**Solutions**:
1. **Check internet connection**:
   - Test with other apps/websites
   - Restart router/modem
   - Contact ISP if needed

2. **Check firewall settings**:
   - Allow Gestura.app through firewall
   - Check corporate firewall restrictions
   - Whitelist api.gestura.app domain

3. **Verify API key**:
   - Settings → Developer → API Keys
   - Generate new key if expired
   - Check key permissions

### Platform-Specific Issues

#### macOS Issues

**Issue**: "Gestura.app is damaged" error
**Solution**:
```bash
# Remove quarantine attribute
sudo xattr -rd com.apple.quarantine /Applications/Gestura.app
```

**Issue**: Accessibility permissions not working
**Solution**:
1. System Preferences → Security & Privacy → Privacy
2. Select "Accessibility"
3. Add Gestura.app to allowed apps
4. Restart Gestura.app

#### Windows Issues

**Issue**: "VCRUNTIME140.dll missing" error
**Solution**:
1. Download Microsoft Visual C++ Redistributable
2. Install both x64 and x86 versions
3. Restart computer

**Issue**: Windows Defender blocking app
**Solution**:
1. Windows Security → Virus & threat protection
2. Add exclusion for Gestura installation folder
3. Or temporarily disable real-time protection

#### Linux Issues

**Issue**: Audio permissions denied
**Solution**:
```bash
# Add user to audio group
sudo usermod -a -G audio $USER

# Restart session
logout # then log back in
```

**Issue**: Bluetooth not working
**Solution**:
```bash
# Install Bluetooth packages
sudo apt install bluetooth bluez bluez-tools

# Start Bluetooth service
sudo systemctl enable bluetooth
sudo systemctl start bluetooth
```

## Advanced Troubleshooting

### Log Analysis

#### Enable Debug Logging
1. Settings → Advanced → Debug Mode
2. Reproduce the issue
3. Settings → Advanced → Export Logs
4. Send logs to support@gestura.app

#### Log Locations
- **macOS**: `~/Library/Logs/Gestura/`
- **Windows**: `%APPDATA%\Gestura\logs\`
- **Linux**: `~/.local/share/gestura/logs/`

#### Reading Logs
```bash
# View recent errors
tail -f ~/.local/share/gestura/logs/error.log

# Search for specific issues
grep -i "bluetooth" ~/.local/share/gestura/logs/debug.log

# View system events
journalctl -u gestura.service
```

### Network Diagnostics

#### Test API Connectivity
```bash
# Test API endpoint
curl -I https://api.gestura.app/v1/health

# Test with authentication
curl -H "Authorization: Bearer YOUR_API_KEY" \
     https://api.gestura.app/v1/status
```

#### Check DNS Resolution
```bash
# Test DNS
nslookup api.gestura.app

# Test with different DNS
nslookup api.gestura.app 8.8.8.8
```

### Hardware Diagnostics

#### Ring Hardware Test
1. Settings → Ring → Hardware Test
2. Follow on-screen instructions
3. Check all sensors (accelerometer, gyroscope, magnetometer)
4. Verify haptic feedback works

#### Microphone Test
1. Settings → Voice → Microphone Test
2. Record 10-second sample
3. Play back and verify quality
4. Check frequency response graph

### Database Issues

#### Corrupt Database
**Symptoms**:
- App crashes on startup
- Settings not saving
- Data loss

**Solution**:
```bash
# Backup current database
cp ~/.local/share/gestura/data.db ~/.local/share/gestura/data.db.backup

# Reset database
rm ~/.local/share/gestura/data.db

# Restart app (will create new database)
gestura
```

#### Database Recovery
```bash
# Try to repair database
sqlite3 ~/.local/share/gestura/data.db ".recover" > recovered.sql
sqlite3 new_data.db < recovered.sql
```

## Getting Help

### Self-Help Resources

1. **Built-in Help**: Press F1 or Help menu
2. **Video Tutorials**: https://youtube.com/gestura
3. **FAQ**: https://gestura.app/faq
4. **Community Forum**: https://community.gestura.app

### Contact Support

#### Before Contacting Support
1. Run diagnostic tool
2. Check this troubleshooting guide
3. Search community forum
4. Try basic solutions (restart, update, etc.)

#### What to Include
- **System Information**: OS version, hardware specs
- **Gestura Version**: Help → About
- **Error Messages**: Exact text of any errors
- **Steps to Reproduce**: What you were doing when issue occurred
- **Log Files**: Export from Settings → Advanced → Export Logs

#### Support Channels
- **Email**: support@gestura.app
- **Live Chat**: Available in app (premium users)
- **Community Forum**: https://community.gestura.app
- **Discord**: https://discord.gg/gestura

#### Response Times
- **Critical Issues**: 4 hours
- **Standard Issues**: 24 hours
- **General Questions**: 48 hours
- **Feature Requests**: 1 week

### Emergency Recovery

#### Complete Reset
If all else fails, perform a complete reset:

1. **Backup important data**:
   - Export custom gestures
   - Save voice training data
   - Note down settings

2. **Uninstall completely**:
   ```bash
   # macOS
   rm -rf /Applications/Gestura.app
   rm -rf ~/Library/Application\ Support/Gestura
   
   # Windows
   # Use Control Panel → Uninstall Programs
   # Then delete %APPDATA%\Gestura
   
   # Linux
   sudo apt remove gestura
   rm -rf ~/.local/share/gestura
   ```

3. **Clean install**:
   - Download latest version
   - Install fresh
   - Go through setup wizard again

4. **Restore data**:
   - Import backed up gestures
   - Retrain voice model
   - Reconfigure settings

---

## Prevention Tips

### Regular Maintenance
- **Update regularly**: Check for updates weekly
- **Clear cache monthly**: Settings → Advanced → Clear Cache
- **Backup settings**: Export settings before major changes
- **Monitor performance**: Check CPU/memory usage periodically

### Best Practices
- **Use quality hardware**: Good microphone and Bluetooth adapter
- **Keep drivers updated**: Especially audio and Bluetooth
- **Maintain clean environment**: Reduce background noise and interference
- **Regular calibration**: Recalibrate gestures monthly

### System Optimization
- **Close unused apps**: Free up system resources
- **Regular restarts**: Restart computer and Gestura.app daily
- **Disk cleanup**: Keep sufficient free disk space
- **Antivirus exclusions**: Add Gestura to antivirus whitelist

---

*If you continue to experience issues after following this guide, please contact our support team with detailed information about your problem.*
