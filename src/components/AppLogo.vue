<script setup lang="ts">
import { useId } from "vue";

withDefaults(
  defineProps<{
    size?: number;
    shining?: boolean;
  }>(),
  { size: 24, shining: false }
);

const uid = useId();
const hexBorderId = `${uid}-hex-border`;
const hexChevId = `${uid}-hex-chev`;
const hexShineGradId = `${uid}-hex-shine-grad`;
const hexShineMaskId = `${uid}-hex-shine-mask`;
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 64 64"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    class="vaix-hex-logo"
  >
    <defs>
      <!-- Hexagone extérieur dégradé bleu -> violet (AI) réactif au thème -->
      <linearGradient :id="hexBorderId" x1="8" y1="6" x2="56" y2="58" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="var(--vaix-hex-1, #6EE7FF)" />
        <stop offset="48%" stop-color="var(--vaix-hex-2, #538BF6)" />
        <stop offset="100%" stop-color="var(--vaix-hex-3, #7C3AED)" />
      </linearGradient>

      <!-- Chevron > aux 5 couleurs du logo (cyan -> bleu -> violet -> rouge -> orange) -->
      <linearGradient :id="hexChevId" x1="23.5" y1="19" x2="43.5" y2="38" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#6EE7FF" />
        <stop offset="28%" stop-color="#538BF6" />
        <stop offset="54%" stop-color="#8B5CF6" />
        <stop offset="78%" stop-color="#E11D48" />
        <stop offset="100%" stop-color="#FB8C1A" />
      </linearGradient>

      <!-- Faisceau lumineux spéculaire pour le reflet au hover -->
      <linearGradient :id="hexShineGradId" x1="0%" y1="0%" x2="100%" y2="0%">
        <stop offset="0%" stop-color="#FFFFFF" stop-opacity="0" />
        <stop offset="32%" stop-color="#FFFFFF" stop-opacity="0.22" />
        <stop offset="50%" stop-color="#FFFFFF" stop-opacity="0.88" />
        <stop offset="68%" stop-color="#FFFFFF" stop-opacity="0.22" />
        <stop offset="100%" stop-color="#FFFFFF" stop-opacity="0" />
      </linearGradient>

      <!-- Masque exact limité à l'hexagone et au chevron -->
      <mask :id="hexShineMaskId" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">
        <path
          d="M32 4.5 L55.82 18.25 V45.75 L32 59.5 L8.18 45.75 V18.25 Z"
          stroke="#FFFFFF"
          stroke-width="3.6"
          stroke-linejoin="round"
          fill="none"
        />
        <path
          d="M23.5 19 L43.5 29.5 L43.5 34.5 L23.5 45 L23.5 37.55 L34.07 32 L23.5 26.45 Z"
          fill="#FFFFFF"
        />
      </mask>
    </defs>

    <!-- 1. Contour hexagonal -->
    <path
      d="M32 4.5 L55.82 18.25 V45.75 L32 59.5 L8.18 45.75 V18.25 Z"
      :stroke="`url(#${hexBorderId})`"
      stroke-width="3.6"
      stroke-linejoin="round"
    />

    <!-- 2. Chevron > centré optiquement -->
    <path
      d="M23.5 19 L43.5 29.5 L43.5 34.5 L23.5 45 L23.5 37.55 L34.07 32 L23.5 26.45 Z"
      :fill="`url(#${hexChevId})`"
    />

    <!-- 3. Couche reflet masquée sur le tracé -->
    <g :mask="`url(#${hexShineMaskId})`">
      <g class="hex-shine-beam" :class="{ 'is-shining': shining }">
        <rect
          x="-28"
          y="-12"
          width="56"
          height="88"
          transform="skewX(-24)"
          :fill="`url(#${hexShineGradId})`"
        />
      </g>
    </g>
  </svg>
</template>

<style scoped>
.vaix-hex-logo {
  display: block;
}

.hex-shine-beam {
  transform: translateX(-64px);
  opacity: 0;
  pointer-events: none;
  will-change: transform, opacity;
}

.hex-shine-beam.is-shining {
  animation: vaixHexShine 820ms cubic-bezier(0.25, 0.1, 0.25, 1) forwards;
}

@keyframes vaixHexShine {
  0% {
    transform: translateX(-64px);
    opacity: 1;
  }
  100% {
    transform: translateX(363px);
    opacity: 1;
  }
}

@media (prefers-reduced-motion: reduce) {
  .hex-shine-beam.is-shining {
    animation: none;
    opacity: 0;
  }
}
</style>
