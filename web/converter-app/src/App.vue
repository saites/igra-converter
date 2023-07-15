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
const editArea = ref(null)
const highlightArea = ref(null)
const highlightHeight = ref(null)

onMounted(() => {
  highlightHeight.value = editArea.style?.height
})

async function validate() {
  errMessage.value = null; 
  validating.value = true;
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

      if (!response.ok) {
        const errObj = await response.json();
        throw new Error(errObj["error"])
      } 

      validationResult.value = await response.json()
    } catch(error) {
      errMessage.value = "" + error;
      validationResult.value = null; 
    }
     
    validating.value = false;
}

async function generate() {
  errMessage.value = null; 
  generating.value = true;
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

    if (!response.ok) {
      const errObj = await response.json();
      throw new Error(errObj["error"])
    } 
    
    registrationData.value = JSON.stringify(await response.json(), null, 2)
    generating.value = false;
    await validate();
  } catch(error) {
    errMessage.value = "" + error;
    registrationData.value = ""; 
  }
   
  generating.value = false;
}

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
           placeholder="Paste JSON registration entries here or click 'Generate' to generate some random data."
      ></textarea>
      
      <pre 
           id="highlightArea" 
           ref="highlightArea"
           :style="highlightHeight ? {'height': highlightHeight + 'px'} : {'height': 'h-96'}"
           aria-hidden="true"
       ><code class="hljs json" language="json" v-html="highlighted"></code></pre>
    </div>

      <div class="flex justify-evenly">
        <button :disabled="generating || validating" @click="generate">
          <svg v-if="generating" 
            class="inline animate-spin h-5 w-5 text-white" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" 
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
          </svg>
          <template v-else>Generate</template>
        </button>
        <button :disabled="generating || validating" @click="validate">
          <svg v-if="validating" 
            class="inline animate-spin h-5 w-5 text-white" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" 
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
          </svg>
          <template v-else>Validate</template>
        </button>
      </div>
  </section>

  <div v-if="errMessage !== null">
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
    @apply rounded bg-indigo-500 hover:bg-indigo-600 text-white p-2 w-24 h-16;
    @apply disabled:bg-gray-500;
}

#editArea, #highlightArea {
  @apply w-full h-96 overflow-y-scroll;
  border: 0;
  margin: 10px;
  display: block;
  overflow-x: auto;
  padding: 0;
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
  background-color: #0d1117;
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
