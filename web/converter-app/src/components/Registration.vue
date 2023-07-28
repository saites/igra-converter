<script setup lang="ts">
import MyGrid from './Grid.vue'
import { createApp, ref, computed } from 'vue'

const BASE_URL = "."

const registrationData = ref("")
const validationResult = ref(null)
const errMessage = ref(null)
const validating = ref(false)
const generating = ref(false)
const nEntries = ref(20)

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
      body: JSON.stringify({"num_people": Number(nEntries.value) ?? 10}),
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

</script>

<template>
<div>
  <section class="pb-4">
    <div class="grid">
      <textarea id="editArea" ref="editArea"
           :disabled="generating || validating"
           spellcheck="false"
           v-model="registrationData"
           placeholder="Paste JSON registration entries here or click 'Generate' to generate some random data."
      ></textarea>
      
    </div>
      <div class="flex justify-evenly">
        <label for="nEntries" class="self-center"># Entries: {{nEntries}}</label>
        <input id="nEntries" type="range" v-model.number="nEntries" min="2" max="200">

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
      :results="validationResult?.results ?? []"
           :relevant="validationResult?.relevant ?? {}"
  >
  </my-grid>
</div>

</template>

<style>
button {
  @apply rounded bg-indigo-500 hover:bg-indigo-600 text-white p-2 w-24 h-12;
  @apply disabled:bg-gray-500;
  @apply disabled:cursor-progress;
}

#editArea {
  @apply block h-96 overflow-y-scroll overflow-x-auto resize-y;
  @apply p-2 m-1 rounded-md border-solid border-2 border-sky-500;
  @apply font-mono text-sky-400 caret-red-400 text-base leading-6;
  background-color: #0d1117;
}
</style>
