<script setup lang="ts">
withDefaults(
  defineProps<{
    size?: number;
  }>(),
  { size: 24 }
);
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 64 64"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <defs>
      <linearGradient id="aiderStudioGrad" x1="8" y1="8" x2="56" y2="56" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#818CF8" />
        <stop offset="100%" stop-color="#38BDF8" />
      </linearGradient>
    </defs>
    <path
      d="M32 6L54.5 19V45L32 58L9.5 45V19L32 6Z"
      stroke="url(#aiderStudioGrad)"
      stroke-width="3.5"
      stroke-linejoin="round"
    />
    <path
      d="M22 25L32 35L42 25"
      stroke="#38BDF8"
      stroke-width="3.5"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <path
      d="M26 42H38"
      stroke="#818CF8"
      stroke-width="3.5"
      stroke-linecap="round"
    />
  </svg>
</template>
