#!/usr/bin/env node

const fs = require('fs')
const path = require('path')

console.log('🔍 Validating required assets...')

const requiredAssets = [
  'public/favicon.ico',
  'public/apple-touch-icon.png',
  'public/favicon-32x32.png',
  'public/favicon-16x16.png',
  'public/site.webmanifest'
]

let hasErrors = false

requiredAssets.forEach(asset => {
  const fullPath = path.join(__dirname, '..', asset)
  if (!fs.existsSync(fullPath)) {
    console.error(`❌ Missing required asset: ${asset}`)
    hasErrors = true
  } else {
    console.log(`✅ Found: ${asset}`)
  }
})

// Check for common image formats in public directory
const publicDir = path.join(__dirname, '..', 'public')
if (fs.existsSync(publicDir)) {
  const files = fs.readdirSync(publicDir)
  const imageFiles = files.filter(file => 
    /\.(png|jpg|jpeg|svg|ico|webp)$/i.test(file)
  )
  
  console.log(`\n📁 Found ${imageFiles.length} image assets in public/`)
  imageFiles.forEach(file => {
    const stats = fs.statSync(path.join(publicDir, file))
    const sizeKB = Math.round(stats.size / 1024)
    console.log(`   ${file} (${sizeKB}KB)`)
    
    // Warn about large assets
    if (sizeKB > 500) {
      console.warn(`⚠️  Large asset detected: ${file} (${sizeKB}KB) - consider optimization`)
    }
  })
}

if (hasErrors) {
  console.error('\n❌ Asset validation failed!')
  process.exit(1)
} else {
  console.log('\n✅ All required assets present!')
}
