<script setup lang="ts">
import MyGrid from './components/Grid.vue'
import { createApp, ref, watchEffect, watch } from 'vue'

const BASE_URL = "http://localhost:8080"

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

</script>

<template>
<div class="flex flex-col items-center p-2">
  <section class="w-5/6">
      <textarea 
      class="w-full h-96"
      :disabled="generating || validating" v-model="registrationData"></textarea>
      <div class="flex justify-evenly">
        <button 
        :class="btn"
        :disabled="generating || validating" @click="generate">Generate</button>
        <button 
        :class="btn"
        :disabled="generating || validating" @click="validate">Validate</button>
      </div>
  </section>
  <div v-if="errMessage">
      {{errMessage}}
  </div>
  <my-grid v-else class="w-5/6"
      :results="validationResult?.results"
      :relevant="validationResult?.relevant"
  >
  </my-grid>
</div>

</template>

