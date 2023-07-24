<script setup lang="ts">
import { createApp, ref, computed } from 'vue'

const BASE_URL = "."

const name = ref("")
const searchResult = ref(null)
const errMessage = ref(null)
const searching = ref(false)

async function search() {
  if (!name?.value) { return }

  errMessage.value = null; 
  searching.value = true;

  try {
    const response = await fetch(`${BASE_URL}/search`, {
      method: "POST",
      cache: "no-cache", 
      headers: {
        "Content-Type": "application/json",
      },
      referrerPolicy: "no-referrer", 
      body: JSON.stringify({"performance_name": name.value }),
    });

    if (!response.ok) {
      const errObj = await response.json();
      throw new Error(errObj["error"])
    } 

    searchResult.value = await response.json()
  } catch(error) {
    errMessage.value = "" + error;
    searchResult.value = null; 
  }
     
  searching.value = false;
}

</script>

<template>
<div>
  <section class="pb-4">
    <div class="flex w-full justify-center">
        <label class="text-end pe-2 " for="performance-name">Name: </label>
        <input id="performance-name" v-model="name">
    </div>

        <button @click="search">Find</button>

    <div v-if="errMessage !== null">
        {{errMessage}}
    </div>

    <div v-else>
      <span>Searching for "{{name}}"</span>
      <div>
        {{searchResult}}
      </div>
    </div>

  </section>
</div>

</template>

