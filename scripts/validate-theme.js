#!/usr/bin/env node

const fs = require('fs')
const path = require('path')

console.log('🎨 Validating theme configuration...')

let hasErrors = false

// Check layout.tsx for correct theme default
const layoutPath = path.join(__dirname, '..', 'src/app/layout.tsx')
if (fs.existsSync(layoutPath)) {
  const layoutContent = fs.readFileSync(layoutPath, 'utf8')
  
  if (layoutContent.includes('defaultTheme="dark"')) {
    console.error('❌ Default theme should be "light", not "dark"')
    console.error('   Fix: Change defaultTheme="dark" to defaultTheme="light" in src/app/layout.tsx')
    hasErrors = true
  } else if (layoutContent.includes('defaultTheme="light"')) {
    console.log('✅ Default theme correctly set to light mode')
  } else {
    console.warn('⚠️  No explicit defaultTheme found - using system default')
  }
  
  // Check for ThemeProvider
  if (layoutContent.includes('ThemeProvider')) {
    console.log('✅ ThemeProvider found in layout')
  } else {
    console.error('❌ ThemeProvider not found in layout')
    hasErrors = true
  }
} else {
  console.error('❌ Layout file not found: src/app/layout.tsx')
  hasErrors = true
}

// Check CSS variables format
const cssPath = path.join(__dirname, '..', 'src/app/globals.css')
if (fs.existsSync(cssPath)) {
  const cssContent = fs.readFileSync(cssPath, 'utf8')
  
  // Check for RGB format variables
  const rgbPattern = /--\w+:\s*\d+\s+\d+\s+\d+/
  if (rgbPattern.test(cssContent)) {
    console.log('✅ CSS variables use correct RGB format')
  } else {
    console.error('❌ CSS variables should use RGB format (e.g., "255 255 255")')
    console.error('   Fix: Convert hex colors to RGB space-separated format')
    hasErrors = true
  }
  
  // Check for both light and dark theme definitions
  if (cssContent.includes(':root {') && cssContent.includes('.dark {')) {
    console.log('✅ Both light and dark theme definitions found')
  } else {
    console.error('❌ Missing theme definitions in CSS')
    hasErrors = true
  }
} else {
  console.error('❌ Global CSS file not found: src/app/globals.css')
  hasErrors = true
}

// Check Tailwind config
const tailwindPath = path.join(__dirname, '..', 'tailwind.config.js')
if (fs.existsSync(tailwindPath)) {
  const tailwindContent = fs.readFileSync(tailwindPath, 'utf8')
  
  if (tailwindContent.includes('darkMode:')) {
    console.log('✅ Dark mode configuration found in Tailwind config')
  } else {
    console.warn('⚠️  No dark mode configuration in Tailwind config')
  }
  
  // Check for CSS variable usage
  if (tailwindContent.includes('var(--')) {
    console.log('✅ CSS variables integrated with Tailwind')
  } else {
    console.warn('⚠️  CSS variables not found in Tailwind config')
  }
} else {
  console.error('❌ Tailwind config not found: tailwind.config.js')
  hasErrors = true
}

if (hasErrors) {
  console.error('\n❌ Theme validation failed!')
  process.exit(1)
} else {
  console.log('\n✅ Theme configuration is valid!')
}
