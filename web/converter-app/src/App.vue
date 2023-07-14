<script setup lang="ts">
import MyGrid from './components/Grid.vue'
import { 
  createApp,
  ref, computed,
  onMounted,
  watchEffect, watch 
} from 'vue'
import { useResizeObserver } from '@vueuse/core'

import hljs from 'highlight.js'
import json from 'highlight.js/lib/languages/json';
hljs.registerLanguage('json', json);
import 'highlight.js/styles/github-dark.css'


const BASE_URL = "."

const registrationData = ref("")
const validationResult = ref(null)
const errMessage = ref(null)
const validating = ref(false)
const generating = ref(false)

function validate() {
  validating.value = true;
}

function generate() {
  generating.value = true;
}

function escapeHTML(html) {
  return html
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#x27;')
}

watch(validating,
    async () => {
      try {
        const response = await fetch(`${BASE_URL}/validate`, {
          method: "POST",
          cache: "no-cache", 
          headers: {
            "Content-Type": "application/json",
          },
          referrerPolicy: "no-referrer", 
          body: registrationData.value,
        });

        validationResult.value = await response.json()
      } catch(error) {
        errMessage.value = "Error: " + error;
        validationResult.value = null; 
      }
       
       validating.value = false;
    }, { immediate: false }
)

watch(generating,
    async () => {
      try {
        const response = await fetch(`${BASE_URL}/generate`, {
          method: "POST",
          cache: "no-cache", 
          headers: {
            "Content-Type": "application/json",
          },
          referrerPolicy: "no-referrer", 
          // body: JSON.stringify(generationOptions.value),
        });

        registrationData.value = JSON.stringify(await response.json(), null, 2)
        // validating.value = true;
      } catch(error) {
        errMessage.value = "Error: " + error;
        registrationData.value = ""; 
      }
       
      generating.value = false;
    }, { immediate: true }
)

const btn = ref(
    'rounded-med bg-indigo-500 hover:bg-indigo-600 text-white p-2'
)

const highlighted = computed(() => {
  const hl = hljs.highlight(registrationData.value, { 
    language: "json",
    // ignoreIllegals: true 
  })
  return hl.value
})

function update(e) {
  registrationData.value = e.target.value
  syncScroll(e)
}

const editArea = ref(null)
const highlightArea = ref(null)
const highlightHeight = ref(null)
onMounted(() => {
  highlightHeight.value = editArea.style?.height
})

function syncScroll(e) {
  if (!highlightArea.value) { return }
  highlightArea.value.scrollTop = e.target.scrollTop
  highlightArea.value.scrollLeft = e.target.scrollLeft
}

useResizeObserver(editArea, (entries) => {
  const entry = entries[0]
  const { width, height } = entry.contentRect
  highlightHeight.value = height 
})

</script>

<template>
<div id="main">
  <section>
    <div class="grid">
      <textarea id="editArea" ref="editArea"
           :disabled="generating || validating"
           spellcheck="false"
           :value="registrationData"
           @input="update"
           @scroll.passive="syncScroll"
      ></textarea>
      
      <pre 
           id="highlightArea" 
           ref="highlightArea"
           :style="highlightHeight ? {'height': highlightHeight + 'px'} : {'height': 'h-96'}"
           aria-hidden="true"
       ><code class="hljs json" language="json" v-html="highlighted"></code></pre>
    </div>

      <div class="flex justify-evenly">
        <button 
        :disabled="generating || validating" @click="generate">Generate</button>
        <button 
        :disabled="generating || validating" @click="validate">Validate</button>
      </div>
  </section>

  <div v-if="errMessage">
      {{errMessage}}
  </div>

  <my-grid v-else 
      :results="validationResult?.results"
      :relevant="validationResult?.relevant"
  >
  </my-grid>
</div>

</template>

<style scoped>
#main {
  @apply grid grid-cols-1 justify-items-center;
}

#main > * {
  @apply max-w-screen-md w-full;
}

button {
    @apply rounded bg-indigo-500 hover:bg-indigo-600 text-white p-2;
    @apply disabled:bg-gray-500;
}

#editArea, #highlightArea {
  @apply w-full h-96 overflow-y-scroll;
  border: 0;
  margin: 10px;
  display: block;
  overflow-x: auto;
  padding: 0.5em;
}

#editArea, #highlightArea, #highlightArea * {
  @apply  text-base leading-6;
  font-family: monospace;
}

pre code.hljs {
  padding: 0!important;
}

#editArea, #highlightArea {
  grid-column: 1;
  grid-row: 1;
}

#editArea {
  @apply resize-y;
  z-index: 1;
  color: transparent;
  background: transparent;
  @apply caret-red-400;
}

#highlightArea {
  z-index: 0;
}

</style>
