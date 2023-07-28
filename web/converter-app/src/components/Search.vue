<script setup lang="ts">
import { ref, watch } from 'vue'
import { debounce } from 'lodash'

const BASE_URL = "."

const name = ref("")
const searchResult = ref(null)
const errMessage = ref(null)
const searching = ref(false)

async function search(newVal, oldVal) {
  if (newVal.trim() === oldVal.trim()) { return }
  if (!name?.value) { return }
  if (name.value.trim() === "") { 
    searchResult.value = null; 
    return 
  }

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


watch(name, search)

</script>

<template>
<div>
  <section class="pb-4">
    <div class="flex w-full justify-center">
        <label class="text-end pe-2 " for="performance-name">Name: </label>
        <input id="performance-name" v-model="name">
    </div>

    <div v-if="errMessage !== null">
        {{errMessage}}
    </div>

    <div v-if="name && searchResult">
      <span class="font-bold">Searching for "{{name.trim()}}"</span>

      <div class="grid grid-cols-7" v-for="p in searchResult.best_matches">
        <span class="col-span-3">{{p.igra_number}} {{p.legal_first}} {{p.legal_last}}<template 
          v-if="p.first_name !== p.legal_first || p.last_name !== p.legal_last"> aka 
          {{p.first_name}} {{p.last_name}}</template>
        </span>
        <span>{{p.sex}}</span>
        <span>{{p.birthdate}}</span>
        <span>{{p.association}}</span>

        <div :hidden="true">
          <div>{{p.division}}</div>
          <div>{{p.status}}</div>
          <div>{{p.ssn}}</div>
          <div>{{p.cell_phone}}</div>
          <div>{{p.home_phone}}</div>
        <span>{{p.address}} {{p.city}}, {{p.state}} {{p.zip}}</span>
        <span>{{p.email}}</span>
        </div>
      </div>

    </div>

  </section>
</div>

</template>

