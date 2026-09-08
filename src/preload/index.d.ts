import type { FrameHostApi } from './index'

declare global {
  interface Window {
    frameHost: FrameHostApi
  }
}

export {}
