<script setup lang="ts">
import { computed, useId } from "vue";

const props = withDefaults(
  defineProps<{
    height?: number;
    layout?: "horizontal" | "stacked";
    showStudio?: boolean;
    shining?: boolean;
  }>(),
  { height: 20, layout: "horizontal", showStudio: true, shining: false }
);

const emit = defineEmits<{
  (e: "shine-end"): void;
}>();

const uid = useId();
const chevGradId = `${uid}-chev`;
const aiGradId = `${uid}-ai`;
const xGradId = `${uid}-x`;
const studioGradId = `${uid}-studio`;
const shineGradId = `${uid}-shine-grad`;
const shineMaskId = `${uid}-shine-mask`;

interface LogoGeometry {
  viewBox: string;
  vbW: number;
  vbH: number;
  chevD: string;
  aiD: string;
  xD: string;
  studioD: string;
  chevGrad: [number, number, number, number];
  aiGrad: [number, number, number, number];
  xGrad: [number, number, number, number];
  studioGrad: [number, number, number, number];
}

const HORIZONTAL_GEOM: LogoGeometry = {
  viewBox: "0 0 2123 266",
  vbW: 2123,
  vbH: 266,
  chevD: "M12.00 60.00 L160.00 138.00 L160.00 174.00 L12.00 252.00 L12.00 197.00 L89.79 156.00 L12.00 115.00 Z",
  aiD: "M354.20 252.00 L335.56 204.88 L242.68 204.88 L225.05 252.00 L162.00 252.00 L262.68 12.00 L318.95 12.00 L418.61 252.00 Z M289.80 76.07 L259.29 158.44 L319.63 158.44 Z M438.61 252.00 L438.61 12.00 L496.92 12.00 L496.92 252.00 Z",
  xD: "M671.83 252.00 L630.58 193.01 L588.17 252.00 L514.92 252.00 L591.64 151.37 L522.24 60.00 L595.88 60.00 L631.73 111.28 L667.98 60.00 L739.30 60.00 L670.29 151.37 L746.24 252.00 Z",
  studioD: "M949.11 109.90 Q941.83 102.31 930.09 97.83 Q918.35 93.36 907.44 93.36 Q901.82 93.36 896.03 94.17 Q890.24 94.98 885.61 97.29 Q880.98 99.59 877.84 103.25 Q874.70 106.92 874.70 112.61 Q874.70 117.49 877.18 120.75 Q879.66 124.00 884.45 126.44 Q889.25 128.88 895.86 130.92 Q902.48 132.95 910.75 135.12 Q922.65 138.37 935.55 142.31 Q948.45 146.24 959.03 152.75 Q969.61 159.25 976.56 168.88 Q983.50 178.51 983.50 192.88 Q983.50 209.42 976.06 221.49 Q968.62 233.56 956.05 241.42 Q943.49 249.29 927.28 253.08 Q911.08 256.88 893.88 256.88 Q868.75 256.88 845.26 249.69 Q821.78 242.51 806.24 229.22 L843.28 198.31 Q851.88 206.98 865.93 212.81 Q879.99 218.64 893.88 218.64 Q900.16 218.64 906.12 217.56 Q912.07 216.47 916.53 214.03 Q921.00 211.59 923.64 207.53 Q926.29 203.46 926.29 197.76 Q926.29 192.34 922.98 188.54 Q919.68 184.75 913.56 181.63 Q907.44 178.51 898.34 175.93 Q889.25 173.36 877.67 170.37 Q866.43 167.39 855.68 163.32 Q844.93 159.25 836.50 152.88 Q828.07 146.51 822.94 137.42 Q817.82 128.34 817.82 115.32 Q817.82 99.32 825.75 87.93 Q833.69 76.54 846.59 69.22 Q859.49 61.90 875.69 58.51 Q891.90 55.12 908.43 55.12 Q928.27 55.12 948.94 61.08 Q969.61 67.05 985.16 78.71 Z M1137.24 99.59 L1137.24 252.00 L1080.69 252.00 L1080.69 99.59 L1014.54 99.59 L1014.54 60.00 L1203.38 60.00 L1203.38 99.59 Z M1443.11 179.59 Q1443.11 196.95 1435.99 211.19 Q1428.88 225.42 1415.66 235.59 Q1402.43 245.76 1383.74 251.46 Q1365.06 257.15 1342.24 257.15 Q1319.09 257.15 1300.57 251.46 Q1282.05 245.76 1269.15 235.59 Q1256.25 225.42 1249.31 211.19 Q1242.36 196.95 1242.36 179.59 L1242.36 60.00 L1298.58 60.00 L1298.58 175.80 Q1298.58 183.66 1301.06 190.71 Q1303.54 197.76 1308.67 203.32 Q1313.80 208.88 1322.23 212.14 Q1330.66 215.39 1342.57 215.39 Q1354.47 215.39 1362.91 212.14 Q1371.34 208.88 1376.63 203.32 Q1381.92 197.76 1384.24 190.71 Q1386.55 183.66 1386.55 175.80 L1386.55 60.00 L1443.11 60.00 Z M1706.31 155.46 Q1706.31 180.68 1694.90 198.98 Q1683.49 217.29 1664.80 229.08 Q1646.12 240.88 1622.64 246.44 Q1599.16 252.00 1575.01 252.00 L1487.71 252.00 L1487.71 60.00 L1572.37 60.00 Q1597.17 60.00 1621.31 64.75 Q1645.46 69.49 1664.31 80.47 Q1683.16 91.46 1694.73 109.90 Q1706.31 128.34 1706.31 155.46 Z M1647.44 155.46 Q1647.44 139.19 1640.99 128.47 Q1634.54 117.76 1623.79 111.39 Q1613.05 105.02 1599.32 102.31 Q1585.60 99.59 1571.05 99.59 L1542.94 99.59 L1542.94 211.86 L1569.72 211.86 Q1584.94 211.86 1598.99 209.02 Q1613.05 206.17 1623.79 199.66 Q1634.54 193.15 1640.99 182.31 Q1647.44 171.46 1647.44 155.46 Z M1750.91 252.00 L1750.91 60.00 L1807.79 60.00 L1807.79 252.00 Z M2110.68 155.19 Q2110.68 177.97 2101.09 196.81 Q2091.50 215.66 2074.47 229.08 Q2057.44 242.51 2033.95 249.83 Q2010.47 257.15 1983.02 257.15 Q1955.57 257.15 1932.26 249.83 Q1908.94 242.51 1891.91 229.08 Q1874.88 215.66 1865.29 196.81 Q1855.70 177.97 1855.70 155.19 Q1855.70 132.14 1865.29 113.56 Q1874.88 94.98 1891.91 81.97 Q1908.94 68.95 1932.26 61.90 Q1955.57 54.85 1983.02 54.85 Q2010.47 54.85 2033.95 61.90 Q2057.44 68.95 2074.47 81.97 Q2091.50 94.98 2101.09 113.56 Q2110.68 132.14 2110.68 155.19 Z M2050.16 155.19 Q2050.16 142.71 2045.20 131.86 Q2040.24 121.02 2031.47 113.29 Q2022.71 105.56 2010.31 101.08 Q1997.91 96.61 1983.02 96.61 Q1968.14 96.61 1955.91 101.08 Q1943.67 105.56 1934.74 113.29 Q1925.81 121.02 1921.02 131.86 Q1916.22 142.71 1916.22 155.19 Q1916.22 168.20 1921.18 179.19 Q1926.14 190.17 1934.91 198.03 Q1943.67 205.90 1955.91 210.37 Q1968.14 214.85 1983.02 214.85 Q1997.91 214.85 2010.14 210.37 Q2022.38 205.90 2031.31 198.03 Q2040.24 190.17 2045.20 179.19 Q2050.16 168.20 2050.16 155.19 Z",
  chevGrad: [12.00, 88.00, 160.00, 236.00],
  aiGrad: [177.00, 12.00, 511.92, 252.00],
  xGrad: [524.92, 60.00, 736.24, 252.00],
  studioGrad: [806.24, 60.00, 2110.68, 252.00],
};

const STACKED_GEOM: LogoGeometry = {
  viewBox: "0 0 815 440",
  vbW: 815,
  vbH: 440,
  chevD: "M40.00 88.00 L188.00 166.00 L188.00 202.00 L40.00 280.00 L40.00 225.00 L117.79 184.00 L40.00 143.00 Z",
  aiD: "M382.20 280.00 L363.56 232.88 L270.68 232.88 L253.05 280.00 L190.00 280.00 L290.68 40.00 L346.95 40.00 L446.61 280.00 Z M317.80 104.07 L287.29 186.44 L347.63 186.44 Z M466.61 280.00 L466.61 40.00 L524.92 40.00 L524.92 280.00 Z",
  xD: "M699.83 280.00 L658.58 221.01 L616.17 280.00 L542.92 280.00 L619.64 179.37 L550.24 88.00 L623.88 88.00 L659.73 139.28 L695.98 88.00 L767.30 88.00 L698.29 179.37 L774.24 280.00 Z",
  studioD: "M114.10 336.09 Q110.33 332.73 104.24 330.75 Q98.15 328.77 92.49 328.77 Q89.57 328.77 86.57 329.13 Q83.57 329.49 81.17 330.51 Q78.77 331.53 77.14 333.15 Q75.51 334.77 75.51 337.29 Q75.51 339.45 76.79 340.90 Q78.08 342.34 80.57 343.42 Q83.05 344.50 86.48 345.40 Q89.91 346.30 94.20 347.26 Q100.38 348.70 107.07 350.44 Q113.76 352.18 119.25 355.06 Q124.73 357.95 128.34 362.21 Q131.94 366.47 131.94 372.83 Q131.94 380.16 128.08 385.50 Q124.22 390.84 117.70 394.33 Q111.18 397.81 102.78 399.49 Q94.37 401.17 85.45 401.17 Q72.42 401.17 60.24 397.99 Q48.06 394.81 40.00 388.92 L59.21 375.24 Q63.67 379.08 70.96 381.66 Q78.25 384.24 85.45 384.24 Q88.71 384.24 91.80 383.76 Q94.89 383.28 97.20 382.20 Q99.52 381.12 100.89 379.32 Q102.26 377.52 102.26 375.00 Q102.26 372.59 100.55 370.91 Q98.83 369.23 95.66 367.85 Q92.49 366.47 87.77 365.33 Q83.05 364.19 77.05 362.87 Q71.22 361.55 65.64 359.75 Q60.07 357.95 55.69 355.12 Q51.32 352.30 48.66 348.28 Q46.00 344.26 46.00 338.49 Q46.00 331.41 50.12 326.37 Q54.24 321.32 60.93 318.08 Q67.62 314.84 76.02 313.34 Q84.43 311.84 93.00 311.84 Q103.29 311.84 114.01 314.48 Q124.73 317.12 132.80 322.28 Z M223.21 331.53 L223.21 399.01 L193.88 399.01 L193.88 331.53 L159.58 331.53 L159.58 314.00 L257.52 314.00 L257.52 331.53 Z M393.39 366.95 Q393.39 374.63 389.70 380.94 Q386.01 387.24 379.15 391.74 Q372.29 396.25 362.60 398.77 Q352.91 401.29 341.07 401.29 Q329.06 401.29 319.46 398.77 Q309.85 396.25 303.16 391.74 Q296.47 387.24 292.87 380.94 Q289.27 374.63 289.27 366.95 L289.27 314.00 L318.43 314.00 L318.43 365.27 Q318.43 368.75 319.72 371.87 Q321.00 375.00 323.66 377.46 Q326.32 379.92 330.69 381.36 Q335.07 382.80 341.24 382.80 Q347.42 382.80 351.79 381.36 Q356.17 379.92 358.91 377.46 Q361.66 375.00 362.86 371.87 Q364.06 368.75 364.06 365.27 L364.06 314.00 L393.39 314.00 Z M541.44 356.26 Q541.44 367.43 535.52 375.54 Q529.60 383.64 519.91 388.86 Q510.22 394.09 498.04 396.55 Q485.86 399.01 473.34 399.01 L428.06 399.01 L428.06 314.00 L471.97 314.00 Q484.83 314.00 497.35 316.10 Q509.88 318.20 519.65 323.07 Q529.43 327.93 535.43 336.09 Q541.44 344.26 541.44 356.26 Z M510.90 356.26 Q510.90 349.06 507.56 344.32 Q504.22 339.57 498.64 336.75 Q493.07 333.93 485.95 332.73 Q478.83 331.53 471.28 331.53 L456.70 331.53 L456.70 381.24 L470.60 381.24 Q478.49 381.24 485.78 379.98 Q493.07 378.72 498.64 375.84 Q504.22 372.95 507.56 368.15 Q510.90 363.35 510.90 356.26 Z M576.11 399.01 L576.11 314.00 L605.61 314.00 L605.61 399.01 Z M774.24 356.14 Q774.24 366.23 769.27 374.57 Q764.29 382.92 755.46 388.86 Q746.62 394.81 734.45 398.05 Q722.27 401.29 708.03 401.29 Q693.79 401.29 681.70 398.05 Q669.61 394.81 660.78 388.86 Q651.94 382.92 646.97 374.57 Q641.99 366.23 641.99 356.14 Q641.99 345.94 646.97 337.71 Q651.94 329.49 660.78 323.73 Q669.61 317.96 681.70 314.84 Q693.79 311.72 708.03 311.72 Q722.27 311.72 734.45 314.84 Q746.62 317.96 755.46 323.73 Q764.29 329.49 769.27 337.71 Q774.24 345.94 774.24 356.14 Z M742.85 356.14 Q742.85 350.62 740.28 345.82 Q737.71 341.02 733.16 337.59 Q728.61 334.17 722.18 332.19 Q715.75 330.21 708.03 330.21 Q700.31 330.21 693.97 332.19 Q687.62 334.17 682.99 337.59 Q678.36 341.02 675.87 345.82 Q673.38 350.62 673.38 356.14 Q673.38 361.91 675.96 366.77 Q678.53 371.63 683.07 375.12 Q687.62 378.60 693.97 380.58 Q700.31 382.56 708.03 382.56 Q715.75 382.56 722.10 380.58 Q728.44 378.60 733.07 375.12 Q737.71 371.63 740.28 366.77 Q742.85 361.91 742.85 356.14 Z",
  chevGrad: [40.00, 116.00, 188.00, 264.00],
  aiGrad: [205.00, 40.00, 539.92, 280.00],
  xGrad: [552.92, 88.00, 764.24, 280.00],
  studioGrad: [40.00, 314.00, 774.24, 399.01],
};

const activeGeom = computed(() =>
  props.layout === "stacked" ? STACKED_GEOM : HORIZONTAL_GEOM
);
</script>

<template>
  <div class="vaix-brand-lockup" :style="{ height: `${height}px` }">
    <svg
      :viewBox="activeGeom.viewBox"
      preserveAspectRatio="xMinYMid meet"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      class="vaix-logo-svg"
      role="img"
      aria-label=">AIx STUDIO"
    >
      <defs>
        <linearGradient
          :id="chevGradId"
          gradientUnits="userSpaceOnUse"
          :x1="activeGeom.chevGrad[0]"
          :y1="activeGeom.chevGrad[1]"
          :x2="activeGeom.chevGrad[2]"
          :y2="activeGeom.chevGrad[3]"
        >
          <stop offset="0%" stop-color="var(--vaix-chevron-1, #FFFFFF)" />
          <stop offset="55%" stop-color="var(--vaix-chevron-2, #D1D5DB)" />
          <stop offset="100%" stop-color="var(--vaix-chevron-3, #8A94A3)" />
        </linearGradient>

        <linearGradient
          :id="aiGradId"
          gradientUnits="userSpaceOnUse"
          :x1="activeGeom.aiGrad[0]"
          :y1="activeGeom.aiGrad[1]"
          :x2="activeGeom.aiGrad[2]"
          :y2="activeGeom.aiGrad[3]"
        >
          <stop offset="0%" stop-color="#6EE7FF" />
          <stop offset="48%" stop-color="#538BF6" />
          <stop offset="100%" stop-color="#7C3AED" />
        </linearGradient>

        <linearGradient
          :id="xGradId"
          gradientUnits="userSpaceOnUse"
          :x1="activeGeom.xGrad[0]"
          :y1="activeGeom.xGrad[1]"
          :x2="activeGeom.xGrad[2]"
          :y2="activeGeom.xGrad[3]"
        >
          <stop offset="0%" stop-color="#D81B60" />
          <stop offset="48%" stop-color="#EC4054" />
          <stop offset="100%" stop-color="#FB8C1A" />
        </linearGradient>

        <linearGradient
          :id="studioGradId"
          gradientUnits="userSpaceOnUse"
          :x1="activeGeom.studioGrad[0]"
          :y1="activeGeom.studioGrad[1]"
          :x2="activeGeom.studioGrad[2]"
          :y2="activeGeom.studioGrad[3]"
        >
          <stop offset="0%" stop-color="var(--vaix-studio-1, #FFFFFF)" />
          <stop offset="50%" stop-color="var(--vaix-studio-2, #D1D5DB)" />
          <stop offset="100%" stop-color="var(--vaix-studio-3, #8A94A3)" />
        </linearGradient>

        <!-- Faisceau spéculaire de reflet au hover -->
        <linearGradient :id="shineGradId" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stop-color="#FFFFFF" stop-opacity="0" />
          <stop offset="32%" stop-color="#FFFFFF" stop-opacity="0.22" />
          <stop offset="50%" stop-color="#FFFFFF" stop-opacity="0.88" />
          <stop offset="68%" stop-color="#FFFFFF" stop-opacity="0.22" />
          <stop offset="100%" stop-color="#FFFFFF" stop-opacity="0" />
        </linearGradient>

        <!-- Masque strict limité au lettrage actif -->
        <mask
          :id="shineMaskId"
          maskUnits="userSpaceOnUse"
          x="0"
          y="0"
          :width="activeGeom.vbW"
          :height="activeGeom.vbH"
        >
          <path :d="activeGeom.chevD" fill="#FFFFFF" />
          <path :d="activeGeom.aiD" fill="#FFFFFF" />
          <path :d="activeGeom.xD" fill="#FFFFFF" />
          <path v-if="showStudio" :d="activeGeom.studioD" fill="#FFFFFF" />
        </mask>
      </defs>

      <path :d="activeGeom.chevD" :fill="`url(#${chevGradId})`" />
      <path :d="activeGeom.aiD" :fill="`url(#${aiGradId})`" />
      <path :d="activeGeom.xD" :fill="`url(#${xGradId})`" />
      <path v-if="showStudio" :d="activeGeom.studioD" :fill="`url(#${studioGradId})`" />

      <!-- Couche reflet masquée sur le lettrage -->
      <g :mask="`url(#${shineMaskId})`">
        <g
          class="logo-shine-beam"
          :class="{ 'is-shining': shining }"
          @animationend="emit('shine-end')"
        >
          <rect
            x="-266"
            y="-40"
            width="532"
            height="520"
            transform="skewX(-24)"
            :fill="`url(#${shineGradId})`"
          />
        </g>
      </g>
    </svg>
  </div>
</template>

<style scoped>
.vaix-brand-lockup {
  display: inline-flex;
  align-items: center;
  user-select: none;
  min-width: 0;
  max-width: 100%;
}

.vaix-logo-svg {
  height: 100%;
  width: auto;
  max-width: 100%;
  display: block;
}

.logo-shine-beam {
  transform: translateX(-1391px);
  opacity: 0;
  pointer-events: none;
  will-change: transform, opacity;
}

.logo-shine-beam.is-shining {
  animation: vaixLogoShine 820ms cubic-bezier(0.25, 0.1, 0.25, 1) forwards;
}

@keyframes vaixLogoShine {
  0% {
    transform: translateX(-1391px);
    opacity: 1;
  }
  100% {
    transform: translateX(2701px);
    opacity: 1;
  }
}

@media (prefers-reduced-motion: reduce) {
  .logo-shine-beam.is-shining {
    animation: none;
    opacity: 0;
  }
}
</style>
