# Store Assets

This directory holds assets for the Chrome Web Store listing.

## Required files

| File                  | Dimensions | Purpose                           |
|-----------------------|------------|-----------------------------------|
| `screenshot-1.png`    | 1280×800   | Download list screenshot          |
| `promo-small.png`     | 440×280    | Small promotional tile            |
| `icon-128.png`        | 128×128    | Store listing icon                |

## icon-128.png

Copy from `extension/public/icon128.png`:

```powershell
Copy-Item extension\public\icon128.png store-assets\icon-128.png
```

## Screenshots

Capture a screenshot of the Ferric UI with an active download list at 1280×800.

## Promo tile

Create a 440×280 promotional image featuring the Ferric logo and tagline:
"Accelerate your downloads with parallel chunked transfers."
