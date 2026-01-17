import React, { useState, useEffect } from 'react';

interface HelpTopic {
  id: string;
  title: string;
  content: string;
  category: string;
  keywords: string[];
}

const helpTopics: HelpTopic[] = [
  {
    id: 'getting-started',
    title: 'Getting Started',
    category: 'Basics',
    keywords: ['start', 'begin', 'setup', 'first'],
    content: `
# Getting Started with Gestura

Welcome to Gestura! This guide will help you get up and running quickly.

## First Steps
1. **Complete the onboarding wizard** if you haven't already
2. **Configure your voice engine** in the Voice panel
3. **Connect your Haptic Harmony ring** in the Ring panel
4. **Customize your settings** in the Settings panel

## Basic Usage
- Use the navigation buttons to switch between panels
- The status bar shows system health and active connections
- Press F1 at any time to open this help system
    `
  },
  {
    id: 'voice-commands',
    title: 'Voice Commands',
    category: 'Voice',
    keywords: ['voice', 'speech', 'commands', 'microphone'],
    content: `
# Voice Commands

Gestura supports multiple voice processing engines for speech-to-text conversion.

## Supported Engines
- **Local Processing**: Uses Whisper models locally (recommended)
- **OpenAI Whisper API**: Cloud-based processing
- **Mock Engine**: For testing purposes

## Configuration
1. Go to the Voice panel
2. Select your preferred provider
3. Configure model path (for local processing)
4. Test your configuration

## Tips
- Speak clearly and at normal volume
- Ensure your microphone is working
- Local processing keeps your data private
    `
  },
  {
    id: 'haptic-ring',
    title: 'Haptic Harmony Ring',
    category: 'Hardware',
    keywords: ['ring', 'haptic', 'bluetooth', 'gestures'],
    content: `
# Haptic Harmony Ring

The Haptic Harmony ring provides gesture input and haptic feedback.

## Setup
1. Ensure your ring is charged and in pairing mode
2. Go to the Ring panel
3. Click "Scan for Rings"
4. Select your ring and click "Pair Ring"

## Features
- **Gesture Detection**: Tap, double-tap, tilt gestures
- **Haptic Feedback**: Various patterns and intensities
- **Battery Monitoring**: Real-time battery level
- **Firmware Updates**: Over-the-air updates

## Gestures
- **Single Tap**: Quick action trigger
- **Double Tap**: Secondary action
- **Tilt Left/Right**: Navigation
- **Tilt Up/Down**: Volume or intensity control
    `
  },
  {
    id: 'privacy-security',
    title: 'Privacy & Security',
    category: 'Privacy',
    keywords: ['privacy', 'security', 'gdpr', 'data'],
    content: `
# Privacy & Security

Gestura is designed with privacy-first principles.

## Data Processing
- **Local by Default**: Voice processing happens on your device
- **Minimal Data Collection**: Only necessary data is collected
- **User Control**: You control what data is processed

## GDPR Compliance
- **Consent Management**: Granular consent for different data types
- **Data Export**: Export all your data at any time
- **Right to be Forgotten**: Delete all your data
- **Audit Trail**: Complete log of data operations

## Security Features
- **Encryption**: All sensitive data is encrypted
- **Secure Storage**: Uses OS keychain for secrets
- **Agent Sandboxing**: Isolated execution environments
- **Permission System**: Fine-grained access control
    `
  },
  {
    id: 'troubleshooting',
    title: 'Troubleshooting',
    category: 'Support',
    keywords: ['help', 'problem', 'issue', 'error', 'fix'],
    content: `
# Troubleshooting

Common issues and solutions.

## Voice Processing Issues
**Problem**: Voice recognition not working
**Solutions**:
- Check microphone permissions
- Test microphone in system settings
- Try different voice engine
- Check model path (for local processing)

## Ring Connection Issues
**Problem**: Ring not connecting
**Solutions**:
- Ensure ring is charged
- Check Bluetooth is enabled
- Try re-pairing the ring
- Restart the application

## Performance Issues
**Problem**: Application running slowly
**Solutions**:
- Check system resources in status bar
- Close unnecessary agents
- Clear telemetry data
- Restart the application

## Getting Help
- Check the system health in the status bar
- Review telemetry data for errors
- Export logs for technical support
- Visit our documentation website
    `
  },
  {
    id: 'keyboard-shortcuts',
    title: 'Keyboard Shortcuts',
    category: 'Reference',
    keywords: ['keyboard', 'shortcuts', 'hotkeys', 'keys'],
    content: `
# Keyboard Shortcuts

Speed up your workflow with these shortcuts.

## Global Shortcuts
- **F1**: Open help system
- **Ctrl+,**: Open settings
- **Ctrl+R**: Refresh application
- **Ctrl+Q**: Quit application

## Panel Navigation
- **Ctrl+1**: Voice panel
- **Ctrl+2**: Ring panel
- **Ctrl+3**: Settings panel

## Voice Panel
- **Space**: Start/stop recording
- **Enter**: Process recording
- **Ctrl+T**: Test voice engine

## Ring Panel
- **Ctrl+S**: Scan for rings
- **Ctrl+P**: Pair selected ring
- **Ctrl+H**: Send haptic test

## Settings Panel
- **Ctrl+S**: Save settings
- **Ctrl+R**: Reset to defaults
    `
  }
];

const HelpSystem: React.FC<{ isOpen: boolean; onClose: () => void }> = ({ isOpen, onClose }) => {
  const [searchTerm, setSearchTerm] = useState('');
  const [selectedTopic, setSelectedTopic] = useState<HelpTopic | null>(null);
  const [filteredTopics, setFilteredTopics] = useState<HelpTopic[]>(helpTopics);

  useEffect(() => {
    if (searchTerm) {
      const filtered = helpTopics.filter(topic =>
        topic.title.toLowerCase().includes(searchTerm.toLowerCase()) ||
        topic.content.toLowerCase().includes(searchTerm.toLowerCase()) ||
        topic.keywords.some(keyword => keyword.toLowerCase().includes(searchTerm.toLowerCase()))
      );
      setFilteredTopics(filtered);
    } else {
      setFilteredTopics(helpTopics);
    }
  }, [searchTerm]);

  const categories = Array.from(new Set(helpTopics.map(topic => topic.category)));

  const renderMarkdown = (content: string) => {
    // Simple markdown rendering (in a real app, use a proper markdown library)
    return content
      .split('\n')
      .map((line, index) => {
        if (line.startsWith('# ')) {
          return <h1 key={index}>{line.substring(2)}</h1>;
        } else if (line.startsWith('## ')) {
          return <h2 key={index}>{line.substring(3)}</h2>;
        } else if (line.startsWith('### ')) {
          return <h3 key={index}>{line.substring(4)}</h3>;
        } else if (line.startsWith('- ')) {
          return <li key={index}>{line.substring(2)}</li>;
        } else if (line.startsWith('**') && line.endsWith('**')) {
          return <strong key={index}>{line.slice(2, -2)}</strong>;
        } else if (line.trim() === '') {
          return <br key={index} />;
        } else {
          return <p key={index}>{line}</p>;
        }
      });
  };

  if (!isOpen) return null;

  return (
    <div className="help-system-overlay">
      <div className="help-system">
        <div className="help-header">
          <h1>Gestura Help</h1>
          <button className="close-button" onClick={onClose}>×</button>
        </div>

        <div className="help-content">
          <div className="help-sidebar">
            <div className="search-box">
              <input
                type="text"
                placeholder="Search help topics..."
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
              />
            </div>

            <div className="help-topics">
              {categories.map(category => (
                <div key={category} className="topic-category">
                  <h3>{category}</h3>
                  {filteredTopics
                    .filter(topic => topic.category === category)
                    .map(topic => (
                      <div
                        key={topic.id}
                        className={`topic-item ${selectedTopic?.id === topic.id ? 'active' : ''}`}
                        onClick={() => setSelectedTopic(topic)}
                      >
                        {topic.title}
                      </div>
                    ))}
                </div>
              ))}
            </div>
          </div>

          <div className="help-main">
            {selectedTopic ? (
              <div className="topic-content">
                {renderMarkdown(selectedTopic.content)}
              </div>
            ) : (
              <div className="help-welcome">
                <h2>Welcome to Gestura Help</h2>
                <p>Select a topic from the sidebar to get started, or use the search box to find specific information.</p>
                
                <div className="quick-links">
                  <h3>Quick Links</h3>
                  <div className="quick-link-grid">
                    <button 
                      className="quick-link"
                      onClick={() => setSelectedTopic(helpTopics.find(t => t.id === 'getting-started')!)}
                    >
                      🚀 Getting Started
                    </button>
                    <button 
                      className="quick-link"
                      onClick={() => setSelectedTopic(helpTopics.find(t => t.id === 'voice-commands')!)}
                    >
                      🎤 Voice Commands
                    </button>
                    <button 
                      className="quick-link"
                      onClick={() => setSelectedTopic(helpTopics.find(t => t.id === 'haptic-ring')!)}
                    >
                      💍 Haptic Ring
                    </button>
                    <button 
                      className="quick-link"
                      onClick={() => setSelectedTopic(helpTopics.find(t => t.id === 'troubleshooting')!)}
                    >
                      🔧 Troubleshooting
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="help-footer">
          <div className="help-shortcuts">
            <span>Press <kbd>F1</kbd> to toggle help • <kbd>Esc</kbd> to close</span>
          </div>
        </div>
      </div>
    </div>
  );
};

export default HelpSystem;
