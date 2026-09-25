<p align="center"><img src="assets/logo.png" alt="rskycam" ></p>

An all-sky camera for the Raspberry Pi. One binary runs the capture loop
and serves a web dashboard — live view, nightly keograms/star-trails/
timelapses, and an astro overlay grid — with no cloud service and no app
to install. Works with a CSI camera (imx219/rpicam) or a ZWO ASI camera,
and optionally a BME280 environmental sensor over I2C.

## Features

- Live dashboard with current frame, exposure/gain, CPU/RAM/disk, and
  sun/moon altitude
- Automatic day/night capture with auto-exposure
- Per-night keogram, star trails, and day/night timelapse (rendered with
  ffmpeg)
- Astro overlay — alt/az and RA/Dec grids, cardinal directions —
  calibrated to your specific lens and mounting
- RTSP video stream (`rtsp://.../allsky`) for Home Assistant, go2rtc,
  Frigate, or any NVR — off by default, one toggle in Settings
- Focus assist (HFD-based, ASIAir-style) for dialing in sharpness at night
- Dark-frame calibration for ZWO ASI cameras
- Optional BME280 sensor overlay (temperature / humidity / pressure)
- Configurable frame and artifact retention
- Single admin login; everything runs locally on the Pi, no cloud account

## Install on a Raspberry Pi

On a Raspberry Pi OS (64-bit) machine:

```bash
curl -fsSL https://raw.githubusercontent.com/awitwicki/rskycam/main/installer/install.sh | sudo bash
```

This installs `ffmpeg`, downloads the latest release, creates a
`rskycam` system user with data dir `/var/lib/rskycam`, installs the
ZWO udev rule and a hardened systemd service, and starts it. It prints
the dashboard URL when done (port 8080).

## Log in

Open `http://<your-pi-hostname-or-ip>:8080`. Default credentials:

- **Username:** `admin`
- **Password:** `pa$$word!0`

Change the password from **Settings** right after your first login.

## Optional: BME280 sensor

A BME280 (or BMP280) breakout on the Pi's I²C pins adds outdoor
temperature, humidity and pressure to the dashboard and to the overlay.
Four jumper leads, no soldering:

<p align="center">
  <img src="assets/bme280-wiring.svg" width="880"
       alt="BME280 wired to a Raspberry Pi: VIN to pin 1 (3V3), GND to pin 6 (GND), SCL to pin 5 (GPIO3/SCL1), SDA to pin 3 (GPIO2/SDA1)">
</p>

| BME280 | Raspberry Pi 40-pin header |
| ------ | -------------------------- |
| VIN    | pin 1 — 3V3                |
| GND    | pin 6 — GND                |
| SCL    | pin 5 — GPIO3 / SCL1       |
| SDA    | pin 3 — GPIO2 / SDA1       |

**3.3 V only** — never wire VIN to pin 2 or pin 4, those are 5 V.

Enable the bus, then check the sensor answers:

```bash
sudo raspi-config              # Interface Options → I2C → Yes, then reboot
sudo apt install -y i2c-tools
i2cdetect -y 1                 # expect 76 or 77
```

Then switch it on in **Settings → Sensor**. rskycam probes both `0x76`
and `0x77`, so either address strap works; on 6-pin GY-BME280 boards
leave `CSB` and `SDO` unconnected.

If you enabled I²C *after* installing rskycam, re-run the install
command above — it adds the service user to the `i2c` group, and leaves
your data and settings untouched.

---

For updating/uninstalling, local development, running without hardware,
and other technical notes, see [DEVELOPMENT.md](DEVELOPMENT.md).
