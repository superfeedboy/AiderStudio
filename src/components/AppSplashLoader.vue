<script setup lang="ts">
import { onMounted, ref, useId, watch } from "vue";

const props = withDefaults(
  defineProps<{
    loading?: boolean;
    minDuration?: number;
  }>(),
  { loading: true, minDuration: 2200 }
);

const emit = defineEmits<{
  (e: "done"): void;
}>();

const uid = useId();
const hexGradId = `${uid}-hex-grad`;
const chev5ColorId = `${uid}-chev-5c`;
const chevMetalId = `${uid}-chev-metal`;
const wmAiGradId = `${uid}-wm-ai`;
const wmXGradId = `${uid}-wm-x`;
const wmStudioGradId = `${uid}-wm-studio`;
const shineGradId = `${uid}-shine-grad`;
const shineMaskId = `${uid}-shine-mask`;

const isVisible = ref(true);
const isFadingOut = ref(false);

// Timeline states
const chevronRotated = ref(false);
const hexAnticipation = ref(false);
const hexStriking = ref(false);
const hexZoomedOut = ref(false);
const chevronGlued = ref(false);
const chevronIsMetal = ref(false);
const wordmarkRevealed = ref(false);
const isShining = ref(false);
const impactShockwave = ref(false);
const progressPercent = ref(0);
const progressLabel = ref("Initialisation du système...");

onMounted(() => {
  startAnimationSequence();
});

let animDone = false;

function checkCompletion() {
  if (animDone && !props.loading) {
    progressPercent.value = 100;
    progressLabel.value = "Prêt !";
    setTimeout(() => {
      isFadingOut.value = true;
      setTimeout(() => {
        isVisible.value = false;
        emit("done");
      }, 420);
    }, 280);
  }
}

watch(
  () => props.loading,
  (isLoading) => {
    if (!isLoading) {
      checkCompletion();
    }
  }
);

function startAnimationSequence() {
  // Phase 1 : 0ms - Chevron vers le bas (v), 5 couleurs vives
  progressPercent.value = 15;
  progressLabel.value = "Chargement des modules...";

  // Phase 2 : 280ms - Rotation vers la droite (>)
  setTimeout(() => {
    chevronRotated.value = true;
    progressPercent.value = 35;
    progressLabel.value = "Initialisation de l'environnement...";

    // 600ms - Recul d'anticipation (-8px)
    setTimeout(() => {
      hexAnticipation.value = true;
    }, 320);
  }, 280);

  // Phase 3 : 840ms - PERCUSSION & ZOOM OUT INSTANTANÉ DE L'HEXAGONE !
  setTimeout(() => {
    hexAnticipation.value = false;
    hexStriking.value = true;
    impactShockwave.value = true;

    // Pile à la percussion :
    // 1. L'hexagone zoom-out (s'agrandit et disparaît en fondu)
    hexZoomedOut.value = true;

    // 2. Le chevron mute en blanc-gris métallique
    chevronIsMetal.value = true;

    // 3. Le chevron glisse pour venir se coller directement au A de >AIx
    chevronGlued.value = true;

    // 4. AIx STUDIO jaillit et se dévoile vers la droite
    wordmarkRevealed.value = true;

    progressPercent.value = 75;
    progressLabel.value = "Restauration des projets...";

    // Rebond neutre du striker
    setTimeout(() => {
      hexStriking.value = false;
    }, 110);
  }, 840);

  // Phase 4 : 1380ms - Reflet spéculaire traversant >AIx STUDIO
  setTimeout(() => {
    isShining.value = true;
    progressPercent.value = 95;
    progressLabel.value = "Finalisation...";
  }, 1380);

  // Fin du cycle d'animation
  const totalAnimTime = Math.max(props.minDuration, 2400);
  setTimeout(() => {
    animDone = true;
    checkCompletion();
  }, totalAnimTime);
}
</script>

<template>
  <Transition name="splash-fade">
    <div
      v-if="isVisible"
      class="vaix-splash-overlay"
      :class="{ 'is-fading-out': isFadingOut }"
      aria-label="Chargement de >AIx STUDIO"
      role="status"
    >
      <!-- FOND VIVANT : 1. Grille matricielle avec masque radial -->
      <div class="splash-bg-grid" />

      <!-- FOND VIVANT : 2. Anneaux orbitaux architecturaux en rotation lente -->
      <div class="splash-orbital-rings">
        <div class="splash-orbital-ring-outer" />
        <div class="splash-orbital-ring-inner" />
      </div>

      <!-- FOND VIVANT : 3. Onde de choc d'impact qui se propage à la percussion -->
      <div
        class="splash-impact-wave"
        :class="{ 'is-fired': impactShockwave }"
      />

      <!-- FOND VIVANT : 4. Coins techniques HUD studio -->
      <div class="splash-hud-corner splash-hud-tl">
        <span class="splash-hud-cross">+</span>
        <span>&gt;AIx_STUDIO // INIT</span>
      </div>
      <div class="splash-hud-corner splash-hud-tr">
        <span>ENGINE // HYBRID_ACTIVE</span>
        <span class="splash-hud-cross">+</span>
      </div>
      <div class="splash-hud-corner splash-hud-bl">
        <span class="splash-hud-cross">+</span>
        <span>SYSTEM_STATUS // MOUNTING</span>
      </div>
      <div class="splash-hud-corner splash-hud-br">
        <span>BUILD // v0.1.0</span>
        <span class="splash-hud-cross">+</span>
      </div>

      <!-- CENTRE PRINCIPAL DE L'ANIMATION -->
      <div class="vaix-splash-center">
        <!-- LOGO COMPLET UNIFIÉ -->
        <div class="vaix-splash-lockup">
          <!-- 1. HEXAGONE & CHEVRON -->
          <div
            class="splash-hex-wrapper"
            :class="{
              'is-anticipating': hexAnticipation,
              'is-striking': hexStriking,
            }"
          >
            <svg
              width="64"
              height="64"
              viewBox="0 0 64 64"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
              class="splash-hex-svg"
            >
              <defs>
                <!-- Dégradé Hexagone AI : Bleu -> Violet -->
                <linearGradient
                  :id="hexGradId"
                  x1="8"
                  y1="6"
                  x2="56"
                  y2="58"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="var(--vaix-hex-1, #6ee7ff)" />
                  <stop offset="48%" stop-color="var(--vaix-hex-2, #538bf6)" />
                  <stop offset="100%" stop-color="var(--vaix-hex-3, #7c3aed)" />
                </linearGradient>

                <!-- Dégradé Chevron 5 couleurs vives -->
                <linearGradient
                  :id="chev5ColorId"
                  x1="23.5"
                  y1="19"
                  x2="43.5"
                  y2="38"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="#6EE7FF" />
                  <stop offset="28%" stop-color="#538BF6" />
                  <stop offset="54%" stop-color="#8B5CF6" />
                  <stop offset="78%" stop-color="#E11D48" />
                  <stop offset="100%" stop-color="#FB8C1A" />
                </linearGradient>

                <!-- Dégradé Chevron Blanc-Gris Métallique -->
                <linearGradient
                  :id="chevMetalId"
                  x1="23.5"
                  y1="19"
                  x2="43.5"
                  y2="38"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="var(--vaix-chevron-1, #FFFFFF)" />
                  <stop offset="55%" stop-color="var(--vaix-chevron-2, #D1D5DB)" />
                  <stop offset="100%" stop-color="var(--vaix-chevron-3, #8A94A3)" />
                </linearGradient>
              </defs>

              <!-- Contour Hexagone : S'agrandit (zoom-out) et disparaît au moment de l'impact -->
              <path
                d="M32 4.5 L55.82 18.25 V45.75 L32 59.5 L8.18 45.75 V18.25 Z"
                :stroke="`url(#${hexGradId})`"
                stroke-width="3.6"
                stroke-linejoin="round"
                class="splash-hex-ring"
                :class="{ 'is-zoomed-out': hexZoomedOut }"
              />

              <!-- Chevron pivotant & glissant collé à AI -->
              <g
                class="splash-chevron-flipper"
                :class="{
                  'is-rotated': chevronRotated,
                  'is-glued': chevronGlued,
                }"
              >
                <!-- Calque 1 : 5 couleurs vives -->
                <path
                  d="M23.5 19 L43.5 29.5 L43.5 34.5 L23.5 45 L23.5 37.55 L34.07 32 L23.5 26.45 Z"
                  :fill="`url(#${chev5ColorId})`"
                  class="splash-chev-color"
                  :class="{ 'is-hidden': chevronIsMetal }"
                />
                <!-- Calque 2 : Blanc-Gris Métallique -->
                <path
                  d="M23.5 19 L43.5 29.5 L43.5 34.5 L23.5 45 L23.5 37.55 L34.07 32 L23.5 26.45 Z"
                  :fill="`url(#${chevMetalId})`"
                  class="splash-chev-metal"
                  :class="{ 'is-visible': chevronIsMetal }"
                />
              </g>
            </svg>
          </div>

          <!-- 2. MOT "AIx STUDIO" (Dévoilé à l'impact, collé au chevron) -->
          <div
            class="splash-wordmark-wrap"
            :class="{ 'is-revealed': wordmarkRevealed }"
          >
            <svg
              viewBox="160 0 1963 266"
              class="splash-wordmark-svg"
              preserveAspectRatio="xMinYMid meet"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
            >
              <defs>
                <linearGradient
                  :id="wmAiGradId"
                  x1="177"
                  y1="12"
                  x2="511.92"
                  y2="252"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="#6EE7FF" />
                  <stop offset="48%" stop-color="#538BF6" />
                  <stop offset="100%" stop-color="#7C3AED" />
                </linearGradient>

                <linearGradient
                  :id="wmXGradId"
                  x1="524.92"
                  y1="60"
                  x2="736.24"
                  y2="252"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="#D81B60" />
                  <stop offset="48%" stop-color="#EC4054" />
                  <stop offset="100%" stop-color="#FB8C1A" />
                </linearGradient>

                <linearGradient
                  :id="wmStudioGradId"
                  x1="806.24"
                  y1="60"
                  x2="2110.68"
                  y2="252"
                  gradientUnits="userSpaceOnUse"
                >
                  <stop offset="0%" stop-color="#FFFFFF" />
                  <stop offset="50%" stop-color="#D1D5DB" />
                  <stop offset="100%" stop-color="#8A94A3" />
                </linearGradient>
              </defs>

              <!-- Lettres AI -->
              <path
                d="M354.20 252.00 L335.56 204.88 L242.68 204.88 L225.05 252.00 L162.00 252.00 L262.68 12.00 L318.95 12.00 L418.61 252.00 Z M289.80 76.07 L259.29 158.44 L319.63 158.44 Z M438.61 252.00 L438.61 12.00 L496.92 12.00 L496.92 252.00 Z"
                :fill="`url(#${wmAiGradId})`"
              />
              <!-- Lettre x -->
              <path
                d="M671.83 252.00 L630.58 193.01 L588.17 252.00 L514.92 252.00 L591.64 151.37 L522.24 60.00 L595.88 60.00 L631.73 111.28 L667.98 60.00 L739.30 60.00 L670.29 151.37 L746.24 252.00 Z"
                :fill="`url(#${wmXGradId})`"
              />
              <!-- Mot STUDIO -->
              <path
                d="M949.11 109.90 Q941.83 102.31 930.09 97.83 Q918.35 93.36 907.44 93.36 Q901.82 93.36 896.03 94.17 Q890.24 94.98 885.61 97.29 Q880.98 99.59 877.84 103.25 Q874.70 106.92 874.70 112.61 Q874.70 117.49 877.18 120.75 Q879.66 124.00 884.45 126.44 Q889.25 128.88 895.86 130.92 Q902.48 132.95 910.75 135.12 Q922.65 138.37 935.55 142.31 Q948.45 146.24 959.03 152.75 Q969.61 159.25 976.56 168.88 Q983.50 178.51 983.50 192.88 Q983.50 209.42 976.06 221.49 Q968.62 233.56 956.05 241.42 Q943.49 249.29 927.28 253.08 Q911.08 256.88 893.88 256.88 Q868.75 256.88 845.26 249.69 Q821.78 242.51 806.24 229.22 L843.28 198.31 Q851.88 206.98 865.93 212.81 Q879.99 218.64 893.88 218.64 Q900.16 218.64 906.12 217.56 Q912.07 216.47 916.53 214.03 Q921.00 211.59 923.64 207.53 Q926.29 203.46 926.29 197.76 Q926.29 192.34 922.98 188.54 Q919.68 184.75 913.56 181.63 Q907.44 178.51 898.34 175.93 Q889.25 173.36 877.67 170.37 Q866.43 167.39 855.68 163.32 Q844.93 159.25 836.50 152.88 Q828.07 146.51 822.94 137.42 Q817.82 128.34 817.82 115.32 Q817.82 99.32 825.75 87.93 Q833.69 76.54 846.59 69.22 Q859.49 61.90 875.69 58.51 Q891.90 55.12 908.43 55.12 Q928.27 55.12 948.94 61.08 Q969.61 67.05 985.16 78.71 Z M1137.24 99.59 L1137.24 252.00 L1080.69 252.00 L1080.69 99.59 L1014.54 99.59 L1014.54 60.00 L1203.38 60.00 L1203.38 99.59 Z M1443.11 179.59 Q1443.11 196.95 1435.99 211.19 Q1428.88 225.42 1415.66 235.59 Q1402.43 245.76 1383.74 251.46 Q1365.06 257.15 1342.24 257.15 Q1319.09 257.15 1300.57 251.46 Q1282.05 245.76 1269.15 235.59 Q1256.25 225.42 1249.31 211.19 Q1242.36 196.95 1242.36 179.59 L1242.36 60.00 L1298.58 60.00 L1298.58 175.80 Q1298.58 183.66 1301.06 190.71 Q1303.54 197.76 1308.67 203.32 Q1313.80 208.88 1322.23 212.14 Q1330.66 215.39 1342.57 215.39 Q1354.47 215.39 1362.91 212.14 Q1371.34 208.88 1376.63 203.32 Q1381.92 197.76 1384.24 190.71 Q1386.55 183.66 1386.55 175.80 L1386.55 60.00 L1443.11 60.00 Z M1706.31 155.46 Q1706.31 180.68 1694.90 198.98 Q1683.49 217.29 1664.80 229.08 Q1646.12 240.88 1622.64 246.44 Q1599.16 252.00 1575.01 252.00 L1487.71 252.00 L1487.71 60.00 L1572.37 60.00 Q1597.17 60.00 1621.31 64.75 Q1645.46 69.49 1664.31 80.47 Q1683.16 91.46 1694.73 109.90 Q1706.31 128.34 1706.31 155.46 Z M1647.44 155.46 Q1647.44 139.19 1640.99 128.47 Q1634.54 117.76 1623.79 111.39 Q1613.05 105.02 1599.32 102.31 Q1585.60 99.59 1571.05 99.59 L1542.94 99.59 L1542.94 211.86 L1569.72 211.86 Q1584.94 211.86 1598.99 209.02 Q1613.05 206.17 1623.79 199.66 Q1634.54 193.15 1640.99 182.31 Q1647.44 171.46 1647.44 155.46 Z M1750.91 252.00 L1750.91 60.00 L1807.79 60.00 L1807.79 252.00 Z M2110.68 155.19 Q2110.68 177.97 2101.09 196.81 Q2091.50 215.66 2074.47 229.08 Q2057.44 242.51 2033.95 249.83 Q2010.47 257.15 1983.02 257.15 Q1955.57 257.15 1932.26 249.83 Q1908.94 242.51 1891.91 229.08 Q1874.88 215.66 1865.29 196.81 Q1855.70 177.97 1855.70 155.19 Q1855.70 132.14 1865.29 113.56 Q1874.88 94.98 1891.91 81.97 Q1908.94 68.95 1932.26 61.90 Q1955.57 54.85 1983.02 54.85 Q2010.47 54.85 2033.95 61.90 Q2057.44 68.95 2074.47 81.97 Q2091.50 94.98 2101.09 113.56 Q2110.68 132.14 2110.68 155.19 Z M2050.16 155.19 Q2050.16 142.71 2045.20 131.86 Q2040.24 121.02 2031.47 113.29 Q2022.71 105.56 2010.31 101.08 Q1997.91 96.61 1983.02 96.61 Q1968.14 96.61 1955.91 101.08 Q1943.67 105.56 1934.74 113.29 Q1925.81 121.02 1921.02 131.86 Q1916.22 142.71 1916.22 155.19 Q1916.22 168.20 1921.18 179.19 Q1926.14 190.17 1934.91 198.03 Q1943.67 205.90 1955.91 210.37 Q1968.14 214.85 1983.02 214.85 Q1997.91 214.85 2010.14 210.37 Q2022.38 205.90 2031.31 198.03 Q2040.24 190.17 2045.20 179.19 Q2050.16 168.20 2050.16 155.19 Z"
                :fill="`url(#${wmStudioGradId})`"
              />
            </svg>
          </div>

          <!-- 3. CALQUE DU REFLET SPÉCULAIRE SUR LE MOT COMPLET >AIx STUDIO -->
          <svg
            class="splash-shine-overlay"
            preserveAspectRatio="none"
            viewBox="0 0 400 64"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
          >
            <defs>
              <linearGradient
                :id="shineGradId"
                x1="0%"
                y1="0%"
                x2="100%"
                y2="0%"
              >
                <stop offset="0%" stop-color="#FFFFFF" stop-opacity="0" />
                <stop offset="25%" stop-color="#FFFFFF" stop-opacity="0.35" />
                <stop offset="50%" stop-color="#FFFFFF" stop-opacity="0.95" />
                <stop offset="75%" stop-color="#FFFFFF" stop-opacity="0.35" />
                <stop offset="100%" stop-color="#FFFFFF" stop-opacity="0" />
              </linearGradient>

              <!-- Masque unifié strict couvrant le chevron collé et les lettres -->
              <mask
                :id="shineMaskId"
                maskUnits="userSpaceOnUse"
                x="0"
                y="0"
                width="400"
                height="64"
              >
                <!-- Chevron collé (x=15) -->
                <g transform="translate(15, 0)">
                  <path
                    d="M23.5 19 L43.5 29.5 L43.5 34.5 L23.5 45 L23.5 37.55 L34.07 32 L23.5 26.45 Z"
                    fill="#FFFFFF"
                  />
                </g>

                <!-- Tracé des lettres AIx STUDIO (calé à x=27) -->
                <g transform="translate(27, 9) scale(0.18)">
                  <path
                    d="M354.20 252.00 L335.56 204.88 L242.68 204.88 L225.05 252.00 L162.00 252.00 L262.68 12.00 L318.95 12.00 L418.61 252.00 Z M289.80 76.07 L259.29 158.44 L319.63 158.44 Z M438.61 252.00 L438.61 12.00 L496.92 12.00 L496.92 252.00 Z"
                    fill="#FFFFFF"
                  />
                  <path
                    d="M671.83 252.00 L630.58 193.01 L588.17 252.00 L514.92 252.00 L591.64 151.37 L522.24 60.00 L595.88 60.00 L631.73 111.28 L667.98 60.00 L739.30 60.00 L670.29 151.37 L746.24 252.00 Z"
                    fill="#FFFFFF"
                  />
                  <path
                    d="M949.11 109.90 Q941.83 102.31 930.09 97.83 Q918.35 93.36 907.44 93.36 Q901.82 93.36 896.03 94.17 Q890.24 94.98 885.61 97.29 Q880.98 99.59 877.84 103.25 Q874.70 106.92 874.70 112.61 Q874.70 117.49 877.18 120.75 Q879.66 124.00 884.45 126.44 Q889.25 128.88 895.86 130.92 Q902.48 132.95 910.75 135.12 Q922.65 138.37 935.55 142.31 Q948.45 146.24 959.03 152.75 Q969.61 159.25 976.56 168.88 Q983.50 178.51 983.50 192.88 Q983.50 209.42 976.06 221.49 Q968.62 233.56 956.05 241.42 Q943.49 249.29 927.28 253.08 Q911.08 256.88 893.88 256.88 Q868.75 256.88 845.26 249.69 Q821.78 242.51 806.24 229.22 L843.28 198.31 Q851.88 206.98 865.93 212.81 Q879.99 218.64 893.88 218.64 Q900.16 218.64 906.12 217.56 Q912.07 216.47 916.53 214.03 Q921.00 211.59 923.64 207.53 Q926.29 203.46 926.29 197.76 Q926.29 192.34 922.98 188.54 Q919.68 184.75 913.56 181.63 Q907.44 178.51 898.34 175.93 Q889.25 173.36 877.67 170.37 Q866.43 167.39 855.68 163.32 Q844.93 159.25 836.50 152.88 Q828.07 146.51 822.94 137.42 Q817.82 128.34 817.82 115.32 Q817.82 99.32 825.75 87.93 Q833.69 76.54 846.59 69.22 Q859.49 61.90 875.69 58.51 Q891.90 55.12 908.43 55.12 Q928.27 55.12 948.94 61.08 Q969.61 67.05 985.16 78.71 Z M1137.24 99.59 L1137.24 252.00 L1080.69 252.00 L1080.69 99.59 L1014.54 99.59 L1014.54 60.00 L1203.38 60.00 L1203.38 99.59 Z M1443.11 179.59 Q1443.11 196.95 1435.99 211.19 Q1428.88 225.42 1415.66 235.59 Q1402.43 245.76 1383.74 251.46 Q1365.06 257.15 1342.24 257.15 Q1319.09 257.15 1300.57 251.46 Q1282.05 245.76 1269.15 235.59 Q1256.25 225.42 1249.31 211.19 Q1242.36 196.95 1242.36 179.59 L1242.36 60.00 L1298.58 60.00 L1298.58 175.80 Q1298.58 183.66 1301.06 190.71 Q1303.54 197.76 1308.67 203.32 Q1313.80 208.88 1322.23 212.14 Q1330.66 215.39 1342.57 215.39 Q1354.47 215.39 1362.91 212.14 Q1371.34 208.88 1376.63 203.32 Q1381.92 197.76 1384.24 190.71 Q1386.55 183.66 1386.55 175.80 L1386.55 60.00 L1443.11 60.00 Z M1706.31 155.46 Q1706.31 180.68 1694.90 198.98 Q1683.49 217.29 1664.80 229.08 Q1646.12 240.88 1622.64 246.44 Q1599.16 252.00 1575.01 252.00 L1487.71 252.00 L1487.71 60.00 L1572.37 60.00 Q1597.17 60.00 1621.31 64.75 Q1645.46 69.49 1664.31 80.47 Q1683.16 91.46 1694.73 109.90 Q1706.31 128.34 1706.31 155.46 Z M1647.44 155.46 Q1647.44 139.19 1640.99 128.47 Q1634.54 117.76 1623.79 111.39 Q1613.05 105.02 1599.32 102.31 Q1585.60 99.59 1571.05 99.59 L1542.94 99.59 L1542.94 211.86 L1569.72 211.86 Q1584.94 211.86 1598.99 209.02 Q1613.05 206.17 1623.79 199.66 Q1634.54 193.15 1640.99 182.31 Q1647.44 171.46 1647.44 155.46 Z M1750.91 252.00 L1750.91 60.00 L1807.79 60.00 L1807.79 252.00 Z M2110.68 155.19 Q2110.68 177.97 2101.09 196.81 Q2091.50 215.66 2074.47 229.08 Q2057.44 242.51 2033.95 249.83 Q2010.47 257.15 1983.02 257.15 Q1955.57 257.15 1932.26 249.83 Q1908.94 242.51 1891.91 229.08 Q1874.88 215.66 1865.29 196.81 Q1855.70 177.97 1855.70 155.19 Q1855.70 132.14 1865.29 113.56 Q1874.88 94.98 1891.91 81.97 Q1908.94 68.95 1932.26 61.90 Q1955.57 54.85 1983.02 54.85 Q2010.47 54.85 2033.95 61.90 Q2057.44 68.95 2074.47 81.97 Q2091.50 94.98 2101.09 113.56 Q2110.68 132.14 2110.68 155.19 Z M2050.16 155.19 Q2050.16 142.71 2045.20 131.86 Q2040.24 121.02 2031.47 113.29 Q2022.71 105.56 2010.31 101.08 Q1997.91 96.61 1983.02 96.61 Q1968.14 96.61 1955.91 101.08 Q1943.67 105.56 1934.74 113.29 Q1925.81 121.02 1921.02 131.86 Q1916.22 142.71 1916.22 155.19 Q1916.22 168.20 1921.18 179.19 Q1926.14 190.17 1934.91 198.03 Q1943.67 205.90 1955.91 210.37 Q1968.14 214.85 1983.02 214.85 Q1997.91 214.85 2010.14 210.37 Q2022.38 205.90 2031.31 198.03 Q2040.24 190.17 2045.20 179.19 Q2050.16 168.20 2050.16 155.19 Z"
                    fill="#FFFFFF"
                  />
                </g>
              </mask>
            </defs>

            <!-- Faisceau spéculaire traversant >AIx STUDIO -->
            <g :mask="`url(#${shineMaskId})`">
              <rect
                class="splash-shine-beam"
                :class="{ 'is-active': isShining }"
                x="0"
                y="-10"
                width="70"
                height="84"
                :fill="`url(#${shineGradId})`"
              />
            </g>
          </svg>
        </div>

        <!-- JAUGE DE PROGRESSION DISCRÈTE -->
        <div class="vaix-splash-progress-wrap">
          <div class="vaix-splash-progress-track">
            <div
              class="vaix-splash-progress-bar"
              :style="{ width: `${progressPercent}%` }"
            />
          </div>
          <div class="vaix-splash-progress-status">
            <span>{{ progressLabel }}</span>
            <span class="vaix-splash-progress-pct">{{ progressPercent }}%</span>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.vaix-splash-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  background-color: var(--app-bg, #0b0f19);
  background-image: radial-gradient(ellipse 70% 55% at 50% 50%, #111a2e 0%, #080c14 100%);
  user-select: none;
  cursor: default;
  overflow: hidden;
  transition: opacity 0.42s ease, filter 0.42s ease;
}

.vaix-splash-overlay.is-fading-out {
  opacity: 0;
  filter: blur(6px);
  pointer-events: none;
}

.splash-fade-enter-active,
.splash-fade-leave-active {
  transition: opacity 0.42s ease;
}

.splash-fade-enter-from,
.splash-fade-leave-to {
  opacity: 0;
}

/* 1. Grille matricielle fine avec masque radial */
.splash-bg-grid {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-size: 28px 28px;
  background-image: radial-gradient(circle, rgba(255, 255, 255, 0.08) 1px, transparent 1px);
  mask-image: radial-gradient(circle at 50% 50%, rgba(0, 0, 0, 0.8) 15%, transparent 75%);
  -webkit-mask-image: radial-gradient(circle at 50% 50%, rgba(0, 0, 0, 0.8) 15%, transparent 75%);
  opacity: 0.6;
}

/* 2. Cercles orbitaux architecturaux */
.splash-orbital-rings {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  opacity: 0.25;
}

.splash-orbital-ring-outer {
  position: absolute;
  width: 480px;
  height: 480px;
  border-radius: 50%;
  border: 1px dashed rgba(83, 139, 246, 0.35);
  animation: spinSlow 60s linear infinite;
}

.splash-orbital-ring-inner {
  position: absolute;
  width: 320px;
  height: 320px;
  border-radius: 50%;
  border: 1px solid rgba(124, 58, 237, 0.25);
  animation: spinReverse 45s linear infinite;
}

@keyframes spinSlow {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

@keyframes spinReverse {
  from {
    transform: rotate(360deg);
  }
  to {
    transform: rotate(0deg);
  }
}

/* 3. Onde de choc d'impact */
.splash-impact-wave {
  position: absolute;
  width: 120px;
  height: 120px;
  border-radius: 50%;
  border: 1.5px solid rgba(83, 139, 246, 0.6);
  pointer-events: none;
  opacity: 0;
  transform: scale(0.6);
}

.splash-impact-wave.is-fired {
  animation: shockwavePulse 0.75s cubic-bezier(0.1, 0.85, 0.25, 1) forwards;
}

@keyframes shockwavePulse {
  0% {
    opacity: 0.8;
    transform: scale(0.6);
    border-color: #6ee7ff;
  }
  60% {
    opacity: 0.35;
    border-color: #7c3aed;
  }
  100% {
    opacity: 0;
    transform: scale(3.2);
    border-color: transparent;
  }
}

/* 4. Coins HUD techniques */
.splash-hud-corner {
  position: absolute;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-family: var(--font-mono, monospace);
  font-size: 0.65rem;
  letter-spacing: 0.08em;
  color: rgba(255, 255, 255, 0.25);
  pointer-events: none;
  padding: 1.25rem 1.75rem;
}

.splash-hud-cross {
  color: var(--vaix-hex-1, #6ee7ff);
  font-weight: bold;
  opacity: 0.6;
}

.splash-hud-tl {
  top: 0;
  left: 0;
}

.splash-hud-tr {
  top: 0;
  right: 0;
}

.splash-hud-bl {
  bottom: 0;
  left: 0;
}

.splash-hud-br {
  bottom: 0;
  right: 0;
}

/* Contenu central */
.vaix-splash-center {
  position: relative;
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2.5rem;
}

/* Bloc logo horizontal unifié */
.vaix-splash-lockup {
  position: relative;
  display: flex;
  align-items: center;
  gap: 0;
}

/* Wrapper hexagone */
.splash-hex-wrapper {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-right: -4px;
  will-change: transform;
  transition: transform 0.22s cubic-bezier(0.2, 0.85, 0.35, 1.15);
}

.splash-hex-wrapper.is-anticipating {
  transform: translateX(-8px);
}

.splash-hex-wrapper.is-striking {
  transform: translateX(21px);
  transition: transform 0.095s cubic-bezier(0.1, 0.9, 0.2, 1);
}

.splash-hex-svg {
  overflow: visible;
}

/* Anneau hexagonal : zoom out à l'impact */
.splash-hex-ring {
  transform-origin: 32px 32px;
  will-change: transform, opacity;
  transition: transform 0.45s cubic-bezier(0.16, 1, 0.3, 1),
    opacity 0.4s ease-out;
}

.splash-hex-ring.is-zoomed-out {
  transform: scale(1.55);
  opacity: 0;
}

/* Chevron pivotant et glissant */
.splash-chevron-flipper {
  transform-origin: 32px 32px;
  transform: rotate(90deg) translateX(0px);
  will-change: transform;
  transition: transform 0.42s cubic-bezier(0.34, 1.45, 0.64, 1);
}

.splash-chevron-flipper.is-rotated {
  transform: rotate(0deg) translateX(0px);
}

.splash-chevron-flipper.is-glued {
  transform: rotate(0deg) translateX(15px);
  transition: transform 0.22s cubic-bezier(0.2, 0.9, 0.3, 1.15);
}

.splash-chev-color {
  opacity: 1;
  transition: opacity 0.16s ease-out;
}

.splash-chev-color.is-hidden {
  opacity: 0;
}

.splash-chev-metal {
  opacity: 0;
  transition: opacity 0.14s ease-in;
}

.splash-chev-metal.is-visible {
  opacity: 1;
}

/* Wordmark AIx STUDIO */
.splash-wordmark-wrap {
  position: relative;
  display: flex;
  align-items: center;
  overflow: hidden;
  clip-path: inset(0 100% 0 0);
  opacity: 0;
  transform: translateX(-10px);
  will-change: clip-path, transform, opacity;
  transition: clip-path 0.42s cubic-bezier(0.16, 1, 0.3, 1),
    transform 0.42s cubic-bezier(0.18, 1.25, 0.35, 1), opacity 0.18s ease;
}

.splash-wordmark-wrap.is-revealed {
  clip-path: inset(0 0% 0 0);
  opacity: 1;
  transform: translateX(0px);
}

.splash-wordmark-svg {
  height: 44px;
  width: auto;
  max-width: 325px;
}

/* Faisceau spéculaire */
.splash-shine-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: 20;
}

.splash-shine-beam {
  opacity: 0;
  will-change: transform;
}

.splash-shine-beam.is-active {
  animation: shineWordmarkSweep 1.25s ease-in-out forwards;
}

@keyframes shineWordmarkSweep {
  0% {
    transform: translateX(-140px) skewX(-24deg);
    opacity: 0;
  }
  8% {
    opacity: 1;
  }
  68% {
    transform: translateX(290px) skewX(-24deg);
    opacity: 1;
  }
  95% {
    opacity: 0.95;
  }
  100% {
    transform: translateX(480px) skewX(-24deg);
    opacity: 0;
  }
}

/* Jauge de progression */
.vaix-splash-progress-wrap {
  width: 100%;
  max-width: 300px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
}

.vaix-splash-progress-track {
  width: 100%;
  height: 3px;
  background-color: rgba(255, 255, 255, 0.08);
  border-radius: 9999px;
  overflow: hidden;
}

.vaix-splash-progress-bar {
  height: 100%;
  background: linear-gradient(90deg, #6ee7ff 0%, #538bf6 50%, #7c3aed 100%);
  border-radius: 9999px;
  transition: width 0.35s ease;
}

.vaix-splash-progress-status {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 0.7rem;
  color: var(--text-muted, #9ca3af);
  font-family: var(--font-mono, monospace);
}

.vaix-splash-progress-pct {
  font-weight: 600;
  color: var(--vaix-hex-1, #6ee7ff);
}
</style>
