import '@testing-library/jest-dom/vitest'

const ctxStub = {
  fillRect() {}, clearRect() {}, beginPath() {}, closePath() {}, arc() {},
  drawImage() {}, rect() {}, ellipse() {}, strokeRect() {},
  moveTo() {}, lineTo() {}, stroke() {}, fill() {}, clip() {},
  save() {}, restore() {}, translate() {}, rotate() {}, setLineDash() {},
  fillText() {}, measureText: () => ({ width: 0 }),
  createRadialGradient: () => ({ addColorStop() {} }),
}

HTMLCanvasElement.prototype.getContext = function () {
  return ctxStub
} as never

HTMLCanvasElement.prototype.toDataURL = () => 'data:image/png;base64,stub'
