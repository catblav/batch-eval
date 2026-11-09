<script setup>
defineProps({
  name: { type: String, required: true },
  size: { type: [Number, String], default: 16 },
  strokeWidth: { type: [Number, String], default: 1.6 },
});
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    class="shrink-0"
    aria-hidden="true"
  >
    <g v-if="name === 'cat'">
      <path d="M5 10 4 4l4.8 3h6.4L20 4l-1 6v4.5A5.5 5.5 0 0 1 13.5 20h-3A5.5 5.5 0 0 1 5 14.5Z" />
      <path d="M9.5 13h.01M14.5 13h.01" />
    </g>
    <g v-else-if="name === 'play'">
      <path d="M8 5.5 19 12 8 18.5Z" />
    </g>
    <g v-else-if="name === 'plug'">
      <path d="M9 3v6M15 3v6M6 9h12v3a6 6 0 0 1-12 0Z" />
      <path d="M12 18v3" />
    </g>
    <g v-else-if="name === 'sliders'">
      <path d="M4 7h6M14 7h6M4 17h10M18 17h2" />
      <circle cx="12" cy="7" r="2" />
      <circle cx="16" cy="17" r="2" />
    </g>
    <g v-else-if="name === 'database'">
      <ellipse cx="12" cy="5" rx="7" ry="2.5" />
      <path d="M5 5v6c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5V5" />
      <path d="M5 11v6c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5v-6" />
    </g>
    <g v-else-if="name === 'chart'">
      <path d="M4 20V11M10 20V5M16 20v-6M20 20H4" />
    </g>
    <g v-else-if="name === 'plus'">
      <path d="M12 5v14M5 12h14" />
    </g>
    <g v-else-if="name === 'search'">
      <circle cx="11" cy="11" r="6" />
      <path d="M20 20l-4.2-4.2" />
    </g>
    <g v-else-if="name === 'trash'">
      <path d="M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13M10 11v5M14 11v5" />
    </g>
    <g v-else-if="name === 'copy'">
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M15 9V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h4" />
    </g>
    <g v-else-if="name === 'more'">
      <circle cx="5" cy="12" r="1.3" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.3" fill="currentColor" stroke="none" />
      <circle cx="19" cy="12" r="1.3" fill="currentColor" stroke="none" />
    </g>
    <g v-else-if="name === 'chevron-down'">
      <path d="M6 9l6 6 6-6" />
    </g>
    <g v-else-if="name === 'chevron-left'">
      <path d="M15 6l-6 6 6 6" />
    </g>
    <g v-else-if="name === 'chevron-right'">
      <path d="M9 6l6 6-6 6" />
    </g>
    <g v-else-if="name === 'sun'">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </g>
    <g v-else-if="name === 'moon'">
      <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5Z" />
    </g>
    <g v-else-if="name === 'close'">
      <path d="M6 6l12 12M18 6 6 18" />
    </g>
    <g v-else-if="name === 'check'">
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </g>
    <g v-else-if="name === 'alert'">
      <path d="M12 4 21 20H3Z" />
      <path d="M12 10v4M12 17h.01" />
    </g>
    <g v-else-if="name === 'refresh'">
      <path d="M20 12a8 8 0 1 1-2.34-5.66" />
      <path d="M20 4v5h-5" />
    </g>
    <g v-else-if="name === 'download'">
      <path d="M12 4v11M7 10l5 5 5-5M5 20h14" />
    </g>
    <g v-else-if="name === 'upload'">
      <path d="M12 20V9M7 14l5-5 5 5M5 4h14" />
    </g>
    <g v-else-if="name === 'edit'">
      <path d="M4 20l4.5-1 10-10a2.1 2.1 0 0 0-3-3l-10 10L4 20Z" />
      <path d="M13 7l3 3" />
    </g>
    <g v-else-if="name === 'stop'">
      <rect x="7" y="7" width="10" height="10" rx="1" fill="currentColor" stroke="none" />
    </g>
    <g v-else-if="name === 'eye'">
      <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12Z" />
      <circle cx="12" cy="12" r="2.5" />
    </g>
    <g v-else-if="name === 'eye-off'">
      <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12Z" />
      <circle cx="12" cy="12" r="2.5" />
      <path d="M4 4l16 16" />
    </g>
    <g v-else-if="name === 'arrow-right'">
      <path d="M5 12h14M13 6l6 6-6 6" />
    </g>
    <g v-else-if="name === 'file'">
      <path d="M6 3h8l4 4v14H6Z" />
      <path d="M14 3v5h5" />
    </g>
    <g v-else-if="name === 'clock'">
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l3 2" />
    </g>
  </svg>
</template>
